//! The namespace: per-instance views, exposures, and path resolution.
//!
//! Every instance has a *view*: mount points mapping paths to part of another
//! instance's *served tree*. Resolving a path in a view picks the longest
//! mount point that prefixes it, then follows the target's *exposures*
//! (parts of its own view it has spliced into its served tree) until the
//! request lands on the instance that will serve it.

use crate::path::{MountPoint, PathName};
use crate::status::Status;
use std::collections::BTreeMap;

/// Where a path lands: an instance, a path in its served tree, and whether
/// any mount or exposure on the way was read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub instance: usize,
    pub path: PathName,
    pub read_only: bool,
}

/// Exposures may point at instances that expose in turn; a loop is cut off here.
const MAX_HOPS: usize = 32;

#[derive(Debug, Default)]
pub struct Namespace {
    instances: Vec<Instance>,
}

#[derive(Debug, Default, Clone)]
struct Instance {
    view: MountTable,
    /// What the instance has spliced into its served tree.
    exposures: MountTable,
}

/// Mount points, and what each names.
#[derive(Debug, Default, Clone)]
struct MountTable(BTreeMap<MountPoint, Resolved>);

impl MountTable {
    /// The longest mount point that prefixes `path` on a segment boundary,
    /// what it names, and the rest of the path below it: `""` for the mount
    /// point itself, or else starting with `/`.
    fn longest<'p>(&self, path: &'p PathName) -> Option<(&MountPoint, &Resolved, &'p str)> {
        path.mount_points().find_map(|(at, rest)| self.0.get_key_value(at).map(|(at, target)| (at, target, rest)))
    }

    /// What `path` names through this table, if a mount point prefixes it.
    fn resolve(&self, path: &PathName) -> Option<Resolved> {
        self.longest(path).map(|(_, target, rest)| Resolved { path: target.path.join(rest), ..target.clone() })
    }
}

impl Namespace {
    /// A namespace for `count` instances, each with an empty view.
    pub fn new(count: usize) -> Namespace {
        Namespace { instances: vec![Instance::default(); count] }
    }

    /// Puts what `target` names at `at` in `instance`'s view: part of another's
    /// served tree, already resolved.
    pub fn mount_resolved(&mut self, instance: usize, at: &MountPoint, target: Resolved) {
        self.instances[instance].view.0.insert(at.clone(), target);
    }

    /// Splices what `target` names into `instance`'s served tree at `at`.
    pub fn expose_resolved(&mut self, instance: usize, at: &MountPoint, target: Resolved) {
        self.instances[instance].exposures.0.insert(at.clone(), target);
    }

    /// `instance`'s view as it is now: each mount point, in order, and what
    /// it names.
    pub fn view(&self, instance: usize) -> impl Iterator<Item = (&MountPoint, &Resolved)> {
        self.instances[instance].view.0.iter()
    }

    /// What `instance` has spliced into its served tree as it is now.
    pub fn exposures(&self, instance: usize) -> impl Iterator<Item = (&MountPoint, &Resolved)> {
        self.instances[instance].exposures.0.iter()
    }

    /// Resolves `path` in `instance`'s view.
    pub fn resolve(&self, instance: usize, path: &PathName) -> Result<Resolved, Status> {
        let resolved = self.instances[instance].view.resolve(path).ok_or(Status::NOT_FOUND)?;
        self.follow(resolved)
    }

    /// The mount point in `instance`'s view that `path` goes through, and the
    /// rest of the path below it: `/` for the mount point itself.
    pub fn mount_of(&self, instance: usize, path: &PathName) -> Option<(&MountPoint, PathName)> {
        let (at, _, rest) = self.instances[instance].view.longest(path)?;
        Some((at, PathName::root().join(rest)))
    }

    /// Follows exposures from where a request has landed to who serves it.
    fn follow(&self, mut resolved: Resolved) -> Result<Resolved, Status> {
        for _ in 0..MAX_HOPS {
            let Some(exposed) = self.instances[resolved.instance].exposures.resolve(&resolved.path) else {
                return Ok(resolved);
            };
            resolved = Resolved { read_only: resolved.read_only || exposed.read_only, ..exposed };
        }
        Err(Status::INVALID)
    }

    /// Makes what `source` names in `instance`'s view also appear at `at`,
    /// replacing any mount already there.
    pub fn bind(&mut self, instance: usize, source: &PathName, at: &MountPoint, read_only: bool) -> Result<(), Status> {
        let mut target = self.resolve(instance, source)?;
        target.read_only |= read_only;
        self.instances[instance].view.0.insert(at.clone(), target);
        Ok(())
    }

    pub fn unmount(&mut self, instance: usize, at: &MountPoint) -> Result<(), Status> {
        self.instances[instance].view.0.remove(at).map(|_| ()).ok_or(Status::NOT_FOUND)
    }

    /// Splices what `source` names in `instance`'s view into `instance`'s served
    /// tree at `at`. Requests there go straight to `source`.
    pub fn expose(&mut self, instance: usize, at: &MountPoint, source: &PathName, read_only: bool) -> Result<(), Status> {
        let mut target = self.resolve(instance, source)?;
        target.read_only |= read_only;
        self.instances[instance].exposures.0.insert(at.clone(), target);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> PathName {
        PathName::parse(text).unwrap()
    }

    fn at(text: &str) -> MountPoint {
        MountPoint::parse(text).unwrap()
    }

    fn resolved(instance: usize, path: &str, read_only: bool) -> Result<Resolved, Status> {
        Ok(Resolved { instance, path: self::path(path), read_only })
    }

    /// Mounts all of `target`'s served tree at `at` in `instance`'s view.
    fn mount(namespace: &mut Namespace, instance: usize, at: &str, target: usize, read_only: bool) {
        namespace.mount_resolved(instance, &self::at(at), Resolved { instance: target, path: PathName::root(), read_only });
    }

    #[test]
    fn longest_mount_on_segment_boundaries() {
        let mut namespace = Namespace::new(4);
        mount(&mut namespace, 0, "/www", 1, false);
        mount(&mut namespace, 0, "/www/blog", 2, false);
        mount(&mut namespace, 0, "/files", 3, true);
        assert_eq!(namespace.resolve(0, &path("/www")), resolved(1, "/", false));
        assert_eq!(namespace.resolve(0, &path("/www/")), resolved(1, "/", false));
        assert_eq!(namespace.resolve(0, &path("/www/index.html")), resolved(1, "/index.html", false));
        assert_eq!(namespace.resolve(0, &path("/www/docs/")), resolved(1, "/docs/", false));
        assert_eq!(namespace.resolve(0, &path("/www/blog/post/1")), resolved(2, "/post/1", false));
        assert_eq!(namespace.resolve(0, &path("/www/blogger")), resolved(1, "/blogger", false));
        assert_eq!(namespace.resolve(0, &path("/files/a")), resolved(3, "/a", true));
        assert_eq!(namespace.resolve(0, &path("/wwwx")), Err(Status::NOT_FOUND));
        assert_eq!(namespace.resolve(1, &path("/www")), Err(Status::NOT_FOUND), "views are private");
    }

    #[test]
    fn mount_of_names_the_mount_and_the_rest() {
        let mut namespace = Namespace::new(3);
        mount(&mut namespace, 0, "/dev/cons", 1, false);
        mount(&mut namespace, 0, "/", 2, false);
        let mount_of = |path| namespace.mount_of(0, &self::path(path)).map(|(at, rest)| format!("{at} {rest}"));
        assert_eq!(mount_of("/dev/cons").as_deref(), Some("/dev/cons /"));
        assert_eq!(mount_of("/dev/cons/x/").as_deref(), Some("/dev/cons /x/"));
        assert_eq!(mount_of("/dev/consoles").as_deref(), Some("/ /dev/consoles"));
        assert_eq!(mount_of("/").as_deref(), Some("/ /"));
        assert_eq!(namespace.mount_of(1, &path("/x")), None);
    }

    #[test]
    fn root_mounts_catch_everything() {
        let mut namespace = Namespace::new(2);
        mount(&mut namespace, 0, "/", 1, false);
        assert_eq!(namespace.resolve(0, &path("/")), resolved(1, "/", false));
        assert_eq!(namespace.resolve(0, &path("/a/b")), resolved(1, "/a/b", false));
    }

    #[test]
    fn bind_resolves_once_and_only_what_is_visible() {
        let mut namespace = Namespace::new(3);
        mount(&mut namespace, 0, "/files", 1, false);
        assert_eq!(namespace.bind(0, &path("/files/docs"), &at("/d"), true), Ok(()));
        assert_eq!(namespace.resolve(0, &path("/d/x")), resolved(1, "/docs/x", true));
        assert_eq!(namespace.unmount(0, &at("/files")), Ok(()));
        assert_eq!(namespace.resolve(0, &path("/d/x")), resolved(1, "/docs/x", true), "bind took effect when made");
        assert_eq!(namespace.resolve(0, &path("/files/x")), Err(Status::NOT_FOUND));
        assert_eq!(namespace.unmount(0, &at("/files")), Err(Status::NOT_FOUND));
        assert_eq!(namespace.bind(0, &path("/nothing"), &at("/n"), false), Err(Status::NOT_FOUND));
        // Re-binding a read-only name read-write doesn't lift the restriction.
        assert_eq!(namespace.bind(0, &path("/d"), &at("/e"), false), Ok(()));
        assert_eq!(namespace.resolve(0, &path("/e")), resolved(1, "/docs", true));
        assert_eq!(namespace.unmount(0, &at("/d/")), Ok(()), "a trailing / names the same mount point");
        assert_eq!(namespace.resolve(0, &path("/d/x")), Err(Status::NOT_FOUND));
    }

    #[test]
    fn exposures_route_past_the_server() {
        // 0 mounts site (1); site mounts files (2) and exposes it at /static.
        let mut namespace = Namespace::new(3);
        mount(&mut namespace, 0, "/www", 1, false);
        mount(&mut namespace, 1, "/files", 2, false);
        assert_eq!(namespace.expose(1, &at("/static"), &path("/files/public"), true), Ok(()));
        assert_eq!(namespace.resolve(0, &path("/www/static/a.css")), resolved(2, "/public/a.css", true));
        assert_eq!(namespace.resolve(0, &path("/www/staticky")), resolved(1, "/staticky", false));
        assert_eq!(namespace.resolve(0, &path("/www/index.html")), resolved(1, "/index.html", false));
        assert_eq!(namespace.expose(1, &at("/x"), &path("/unseen"), false), Err(Status::NOT_FOUND));
    }

    #[test]
    fn exposure_loops_are_cut_off() {
        let mut namespace = Namespace::new(2);
        mount(&mut namespace, 0, "/b", 1, false);
        mount(&mut namespace, 1, "/a", 0, false);
        namespace.expose(0, &at("/loop"), &path("/b/loop"), false).unwrap();
        namespace.expose(1, &at("/loop"), &path("/a/loop"), false).unwrap();
        assert_eq!(namespace.resolve(0, &path("/b/loop")), Err(Status::INVALID));
    }
}
