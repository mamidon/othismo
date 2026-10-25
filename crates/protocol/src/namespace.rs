//! The namespace: per-instance views, exposures, and path resolution.
//!
//! Every instance has a *view*: mount points mapping paths to part of another
//! instance's *served tree*. Resolving a path in a view picks the longest
//! mount point that prefixes it, then follows the target's *exposures*
//! (parts of its own view it has spliced into its served tree) until the
//! request lands on the instance that will serve it.

use crate::path::{clean, mount_point};
use crate::status::Status;
use std::collections::BTreeMap;

/// The rest of `path` below `prefix` (`""` for an exact match), if `prefix`
/// is a prefix on a segment boundary.
fn below<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix == "/" {
        return Some(if path == "/" { "" } else { path });
    }
    let rest = path.strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

/// `base` followed by `rest`, where `rest` is `""` or starts with `/`.
fn join(base: &str, rest: &str) -> String {
    match (base, rest) {
        (_, "") => base.to_string(),
        ("/", _) => rest.to_string(),
        _ => format!("{base}{rest}"),
    }
}

/// Where a path lands: an instance, a path in its served tree, and whether
/// any mount or exposure on the way was read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub instance: usize,
    pub path: String,
    pub read_only: bool,
}

/// A mount point in a view, or an exposure in a served tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub at: String,
    pub target: Resolved,
}

/// Exposures may point at instances that expose in turn; a loop is cut off here.
const MAX_HOPS: usize = 32;

#[derive(Debug, Default)]
pub struct Namespace {
    views: Vec<Vec<Mount>>,
    exposed: Vec<Vec<Mount>>,
}

impl Namespace {
    /// A namespace for `n` instances, each with an empty view.
    pub fn new(n: usize) -> Namespace {
        Namespace { views: vec![vec![]; n], exposed: vec![vec![]; n] }
    }

    /// Mounts all of `target`'s served tree at `at` in `instance`'s view.
    /// Used to build views from the image, so `at` must already be a [`mount_point`].
    pub fn mount(&mut self, instance: usize, at: &str, target: usize, read_only: bool) {
        let target = Resolved { instance: target, path: "/".into(), read_only };
        put(&mut self.views[instance], at.into(), target);
    }

    /// Puts what `target` names at `at` in `instance`'s view: part of another's
    /// served tree, already resolved. `at` must already be a [`mount_point`].
    pub fn mount_resolved(&mut self, instance: usize, at: &str, target: Resolved) {
        put(&mut self.views[instance], at.into(), target);
    }

    /// Splices what `target` names into `instance`'s served tree at `at`, which
    /// must already be a [`mount_point`].
    pub fn expose_resolved(&mut self, instance: usize, at: &str, target: Resolved) {
        put(&mut self.exposed[instance], at.into(), target);
    }

    /// `instance`'s view as it is now, mount point by mount point.
    pub fn view(&self, instance: usize) -> &[Mount] {
        &self.views[instance]
    }

    /// What `instance` has spliced into its served tree as it is now.
    pub fn exposed(&self, instance: usize) -> &[Mount] {
        &self.exposed[instance]
    }

    /// Resolves `path` in `instance`'s view.
    pub fn resolve(&self, instance: usize, path: &str) -> Result<Resolved, Status> {
        let path = clean(path)?;
        let (m, rest) = longest(&self.views[instance], &path).ok_or(Status::NotFound)?;
        self.follow(Resolved { path: join(&m.target.path, rest), ..m.target.clone() })
    }

    /// The mount point in `instance`'s view that `path` goes through, and the
    /// rest of the path below it: `/` for the mount point itself.
    pub fn mount_of(&self, instance: usize, path: &str) -> Option<(String, String)> {
        let path = clean(path).ok()?;
        let (m, rest) = longest(&self.views[instance], &path)?;
        Some((m.at.clone(), if rest.is_empty() { "/".to_string() } else { rest.to_string() }))
    }

    /// Follows exposures from where a request has landed to who serves it.
    fn follow(&self, mut r: Resolved) -> Result<Resolved, Status> {
        for _ in 0..MAX_HOPS {
            let Some((m, rest)) = longest(&self.exposed[r.instance], &r.path) else { return Ok(r) };
            r = Resolved {
                instance: m.target.instance,
                path: join(&m.target.path, rest),
                read_only: r.read_only || m.target.read_only,
            };
        }
        Err(Status::Invalid)
    }

    /// Makes what `source` names in `instance`'s view also appear at `at`,
    /// replacing any mount already there.
    pub fn bind(&mut self, instance: usize, source: &str, at: &str, read_only: bool) -> Result<(), Status> {
        let mut target = self.resolve(instance, source)?;
        target.read_only |= read_only;
        put(&mut self.views[instance], mount_point(at)?, target);
        Ok(())
    }

    pub fn unmount(&mut self, instance: usize, at: &str) -> Result<(), Status> {
        let at = mount_point(at)?;
        let view = &mut self.views[instance];
        let before = view.len();
        view.retain(|m| m.at != at);
        if view.len() == before { Err(Status::NotFound) } else { Ok(()) }
    }

    /// Splices what `source` names in `instance`'s view into `instance`'s served
    /// tree at `at`. Requests there go straight to `source`.
    pub fn expose(&mut self, instance: usize, at: &str, source: &str, read_only: bool) -> Result<(), Status> {
        let mut target = self.resolve(instance, source)?;
        target.read_only |= read_only;
        put(&mut self.exposed[instance], mount_point(at)?, target);
        Ok(())
    }
}

fn put(mounts: &mut Vec<Mount>, at: String, target: Resolved) {
    mounts.retain(|m| m.at != at);
    mounts.push(Mount { at, target });
}

fn longest<'m, 'p>(mounts: &'m [Mount], path: &'p str) -> Option<(&'m Mount, &'p str)> {
    mounts
        .iter()
        .filter_map(|m| below(path, &m.at).map(|rest| (m, rest)))
        .max_by_key(|(m, _)| m.at.len())
}

/// How an instance may reach another one, at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    pub read_only: bool,
    /// The instance mounted in its own view that the reach goes through;
    /// `None` when the target itself is mounted there.
    pub via: Option<usize>,
}

/// For each instance, every other instance it could ever reach, given the
/// image's mounts: `mounts[i]` lists `(target, read_only)` for `i`'s view.
///
/// Views and exposures only ever hold names their owner could already
/// resolve, so this transitive closure bounds everything bind and expose
/// can do at run time. A target counts as read-write if any path to it is.
pub fn reach(mounts: &[Vec<(usize, bool)>]) -> Vec<BTreeMap<usize, Reach>> {
    (0..mounts.len())
        .map(|start| {
            let mut out: BTreeMap<usize, Reach> = BTreeMap::new();
            // (instance, read-only so far, the mount in start's view this goes through)
            let mut work: Vec<(usize, bool, Option<usize>)> = vec![(start, false, None)];
            let mut seen = vec![[false; 2]; mounts.len()];
            while let Some((at, ro, first)) = work.pop() {
                for &(t, mount_ro) in &mounts[at] {
                    let ro = ro || mount_ro;
                    let first = first.unwrap_or(t);
                    if std::mem::replace(&mut seen[t][ro as usize], true) {
                        continue;
                    }
                    if t != start {
                        let reach = Reach { read_only: ro, via: (first != t).then_some(first) };
                        match out.get_mut(&t) {
                            Some(r) if r.read_only && !ro => *r = reach,
                            Some(_) => {}
                            None => {
                                out.insert(t, reach);
                            }
                        }
                    }
                    work.push((t, ro, Some(first)));
                }
            }
            out
        })
        .collect()
}

/// The order instances start in: each after everything it mounts, and
/// otherwise in image order. Where instances mount each other, the cycle is
/// broken at the one that comes first in the image, which starts last.
pub fn start_order(mounts: &[Vec<usize>]) -> Vec<usize> {
    fn visit(at: usize, mounts: &[Vec<usize>], seen: &mut [bool], out: &mut Vec<usize>) {
        if std::mem::replace(&mut seen[at], true) {
            return;
        }
        for &t in &mounts[at] {
            visit(t, mounts, seen, out);
        }
        out.push(at);
    }
    let mut seen = vec![false; mounts.len()];
    let mut out = Vec::with_capacity(mounts.len());
    for i in 0..mounts.len() {
        visit(i, mounts, &mut seen, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(instance: usize, path: &str, read_only: bool) -> Result<Resolved, Status> {
        Ok(Resolved { instance, path: path.into(), read_only })
    }

    #[test]
    fn longest_mount_on_segment_boundaries() {
        let mut ns = Namespace::new(4);
        ns.mount(0, "/www", 1, false);
        ns.mount(0, "/www/blog", 2, false);
        ns.mount(0, "/files", 3, true);
        assert_eq!(ns.resolve(0, "/www"), r(1, "/", false));
        assert_eq!(ns.resolve(0, "/www/"), r(1, "/", false));
        assert_eq!(ns.resolve(0, "/www/index.html"), r(1, "/index.html", false));
        assert_eq!(ns.resolve(0, "/www/docs/"), r(1, "/docs/", false));
        assert_eq!(ns.resolve(0, "/www/blog/post/1"), r(2, "/post/1", false));
        assert_eq!(ns.resolve(0, "/www/blogger"), r(1, "/blogger", false));
        assert_eq!(ns.resolve(0, "/files/a"), r(3, "/a", true));
        assert_eq!(ns.resolve(0, "/wwwx"), Err(Status::NotFound));
        assert_eq!(ns.resolve(0, "/www/../files/a"), Err(Status::Invalid));
        assert_eq!(ns.resolve(1, "/www"), Err(Status::NotFound), "views are private");
    }

    #[test]
    fn mount_of_names_the_mount_and_the_rest() {
        let mut ns = Namespace::new(3);
        ns.mount(0, "/dev/cons", 1, false);
        ns.mount(0, "/", 2, false);
        let m = |path| ns.mount_of(0, path).map(|(at, rest)| format!("{at} {rest}"));
        assert_eq!(m("/dev/cons").as_deref(), Some("/dev/cons /"));
        assert_eq!(m("/dev/cons/x/").as_deref(), Some("/dev/cons /x/"));
        assert_eq!(m("/dev/consoles").as_deref(), Some("/ /dev/consoles"));
        assert_eq!(m("/").as_deref(), Some("/ /"));
        assert_eq!(ns.mount_of(1, "/x"), None);
        assert_eq!(ns.mount_of(0, "/a/../b"), None);
    }

    #[test]
    fn root_mounts_catch_everything() {
        let mut ns = Namespace::new(2);
        ns.mount(0, "/", 1, false);
        assert_eq!(ns.resolve(0, "/"), r(1, "/", false));
        assert_eq!(ns.resolve(0, "/a/b"), r(1, "/a/b", false));
    }

    #[test]
    fn bind_resolves_once_and_only_what_is_visible() {
        let mut ns = Namespace::new(3);
        ns.mount(0, "/files", 1, false);
        assert_eq!(ns.bind(0, "/files/docs", "/d", true), Ok(()));
        assert_eq!(ns.resolve(0, "/d/x"), r(1, "/docs/x", true));
        assert_eq!(ns.unmount(0, "/files"), Ok(()));
        assert_eq!(ns.resolve(0, "/d/x"), r(1, "/docs/x", true), "bind took effect when made");
        assert_eq!(ns.resolve(0, "/files/x"), Err(Status::NotFound));
        assert_eq!(ns.unmount(0, "/files"), Err(Status::NotFound));
        assert_eq!(ns.bind(0, "/nothing", "/n", false), Err(Status::NotFound));
        assert_eq!(ns.bind(0, "/d", "relative", false), Err(Status::Invalid));
        // Re-binding a read-only name read-write doesn't lift the restriction.
        assert_eq!(ns.bind(0, "/d", "/e", false), Ok(()));
        assert_eq!(ns.resolve(0, "/e"), r(1, "/docs", true));
        assert_eq!(ns.unmount(0, "/d/"), Ok(()), "a trailing / names the same mount point");
        assert_eq!(ns.resolve(0, "/d/x"), Err(Status::NotFound));
    }

    #[test]
    fn exposures_route_past_the_server() {
        // 0 mounts site (1); site mounts files (2) and exposes it at /static.
        let mut ns = Namespace::new(3);
        ns.mount(0, "/www", 1, false);
        ns.mount(1, "/files", 2, false);
        assert_eq!(ns.expose(1, "/static", "/files/public", true), Ok(()));
        assert_eq!(ns.resolve(0, "/www/static/a.css"), r(2, "/public/a.css", true));
        assert_eq!(ns.resolve(0, "/www/staticky"), r(1, "/staticky", false));
        assert_eq!(ns.resolve(0, "/www/index.html"), r(1, "/index.html", false));
        assert_eq!(ns.expose(1, "/x", "/unseen", false), Err(Status::NotFound));
    }

    #[test]
    fn exposure_loops_are_cut_off() {
        let mut ns = Namespace::new(2);
        ns.mount(0, "/b", 1, false);
        ns.mount(1, "/a", 0, false);
        ns.expose(0, "/loop", "/b/loop", false).unwrap();
        ns.expose(1, "/loop", "/a/loop", false).unwrap();
        assert_eq!(ns.resolve(0, "/b/loop"), Err(Status::Invalid));
    }

    #[test]
    fn reach_is_the_closure_of_mounts() {
        // http(0) mounts net(1) and site(2); site mounts files(3) read-only.
        let mounts = vec![vec![(1, false), (2, false)], vec![], vec![(3, true)], vec![]];
        let reach = reach(&mounts);
        let ro = |i: usize, t: usize| reach[i].get(&t).map(|r| (r.read_only, r.via));
        assert_eq!(ro(0, 1), Some((false, None)));
        assert_eq!(ro(0, 2), Some((false, None)));
        assert_eq!(ro(0, 3), Some((true, Some(2))));
        assert_eq!(ro(2, 3), Some((true, None)));
        assert!(reach[1].is_empty());
        assert!(!reach[0].contains_key(&0));
    }

    #[test]
    fn reach_prefers_a_read_write_path() {
        // 0 mounts 1 read-only and 2; 2 mounts 1 read-write.
        let mounts = vec![vec![(1, true), (2, false)], vec![], vec![(1, false)]];
        let reach = reach(&mounts);
        assert_eq!(reach[0][&1], Reach { read_only: false, via: Some(2) });
    }

    #[test]
    fn instances_start_after_what_they_mount() {
        // http(0) mounts site(1) and cons(3); site mounts files(2).
        assert_eq!(start_order(&[vec![1, 3], vec![2], vec![], vec![]]), vec![2, 1, 3, 0]);
        // Unrelated instances keep image order.
        assert_eq!(start_order(&[vec![], vec![], vec![]]), vec![0, 1, 2]);
        // 0 and 1 mount each other; 0 comes first in the image, so starts last.
        assert_eq!(start_order(&[vec![1], vec![0]]), vec![1, 0]);
    }
}
