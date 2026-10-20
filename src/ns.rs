//! The namespace: per-instance views, exposures, and path resolution.
//!
//! Every instance has a *view*: mount points mapping paths to part of another
//! instance's *served tree*. Resolving a path in a view picks the longest
//! mount point that prefixes it, then follows the target's *exposures*
//! (parts of its own view it has spliced into its served tree) until the
//! request lands on the instance that will serve it.

use std::collections::BTreeMap;
use std::fmt;

/// What a request asks for. Safe verbs never change anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Read,
    Stat,
    List,
    Watch,
    Write,
    Create,
    Remove,
    Invoke,
}

pub const VERBS: [Verb; 8] =
    [Verb::Read, Verb::Stat, Verb::List, Verb::Watch, Verb::Write, Verb::Create, Verb::Remove, Verb::Invoke];

impl Verb {
    pub fn from_i32(v: i32) -> Option<Verb> {
        VERBS.get(usize::try_from(v).ok()?).copied()
    }

    pub fn is_safe(self) -> bool {
        matches!(self, Verb::Read | Verb::Stat | Verb::List | Verb::Watch)
    }

    /// The verb's name as written in images: `read`, `write`, ...
    pub fn keyword(self) -> &'static str {
        match self {
            Verb::Read => "read",
            Verb::Stat => "stat",
            Verb::List => "list",
            Verb::Watch => "watch",
            Verb::Write => "write",
            Verb::Create => "create",
            Verb::Remove => "remove",
            Verb::Invoke => "invoke",
        }
    }

    pub fn from_keyword(s: &str) -> Option<Verb> {
        VERBS.into_iter().find(|v| v.keyword() == s)
    }
}

impl fmt::Display for Verb {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// The outcome carried by every reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    NotFound,
    ReadOnly,
    NotSupported,
    IsDirectory,
    NotDirectory,
    Exists,
    Invalid,
    Io,
}

const STATUSES: [Status; 9] = [
    Status::Ok,
    Status::NotFound,
    Status::ReadOnly,
    Status::NotSupported,
    Status::IsDirectory,
    Status::NotDirectory,
    Status::Exists,
    Status::Invalid,
    Status::Io,
];

impl Status {
    pub fn from_i32(v: i32) -> Option<Status> {
        STATUSES.get(usize::try_from(v).ok()?).copied()
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// `Flag.ReadOnly` in the SDK: only safe verbs pass.
pub const READ_ONLY: i32 = 1;

/// Normalizes an absolute path: collapses `//` and `.` segments and keeps a
/// trailing `/`. Relative paths, `..` segments and NUL bytes are `Invalid`.
pub fn clean(path: &str) -> Result<String, Status> {
    if !path.starts_with('/') || path.contains('\0') {
        return Err(Status::Invalid);
    }
    let mut out = String::with_capacity(path.len());
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Err(Status::Invalid),
            seg => {
                out.push('/');
                out.push_str(seg);
            }
        }
    }
    if out.is_empty() || path.ends_with('/') || path.ends_with("/.") {
        out.push('/');
    }
    Ok(out)
}

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
    pub inst: usize,
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

    /// Mounts all of `target`'s served tree at `at` in `inst`'s view.
    /// Used to build views from the image, so `at` must already be clean.
    pub fn mount(&mut self, inst: usize, at: &str, target: usize, read_only: bool) {
        let target = Resolved { inst: target, path: "/".into(), read_only };
        put(&mut self.views[inst], at.into(), target);
    }

    /// Resolves `path` in `inst`'s view.
    pub fn resolve(&self, inst: usize, path: &str) -> Result<Resolved, Status> {
        let path = clean(path)?;
        let (m, rest) = longest(&self.views[inst], &path).ok_or(Status::NotFound)?;
        self.follow(Resolved { path: join(&m.target.path, rest), ..m.target.clone() })
    }

    /// Follows exposures from where a request has landed to who serves it.
    fn follow(&self, mut r: Resolved) -> Result<Resolved, Status> {
        for _ in 0..MAX_HOPS {
            let Some((m, rest)) = longest(&self.exposed[r.inst], &r.path) else { return Ok(r) };
            r = Resolved {
                inst: m.target.inst,
                path: join(&m.target.path, rest),
                read_only: r.read_only || m.target.read_only,
            };
        }
        Err(Status::Invalid)
    }

    /// Makes what `source` names in `inst`'s view also appear at `at`,
    /// replacing any mount already there.
    pub fn bind(&mut self, inst: usize, source: &str, at: &str, read_only: bool) -> Result<(), Status> {
        let mut target = self.resolve(inst, source)?;
        target.read_only |= read_only;
        put(&mut self.views[inst], clean(at)?, target);
        Ok(())
    }

    pub fn unmount(&mut self, inst: usize, at: &str) -> Result<(), Status> {
        let at = clean(at)?;
        let view = &mut self.views[inst];
        let before = view.len();
        view.retain(|m| m.at != at);
        if view.len() == before { Err(Status::NotFound) } else { Ok(()) }
    }

    /// Splices what `source` names in `inst`'s view into `inst`'s served
    /// tree at `at`. Requests there go straight to `source`.
    pub fn expose(&mut self, inst: usize, at: &str, source: &str, read_only: bool) -> Result<(), Status> {
        let mut target = self.resolve(inst, source)?;
        target.read_only |= read_only;
        put(&mut self.exposed[inst], clean(at)?, target);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning_paths() {
        assert_eq!(clean("/").unwrap(), "/");
        assert_eq!(clean("//a//b/./c").unwrap(), "/a/b/c");
        assert_eq!(clean("/a/b/").unwrap(), "/a/b/");
        assert_eq!(clean("/a/.").unwrap(), "/a/");
        assert_eq!(clean("/."), Ok("/".into()));
        assert_eq!(clean("a/b"), Err(Status::Invalid));
        assert_eq!(clean(""), Err(Status::Invalid));
        assert_eq!(clean("/a/../b"), Err(Status::Invalid));
        assert_eq!(clean("/a/.."), Err(Status::Invalid));
        assert_eq!(clean("/a\0"), Err(Status::Invalid));
        assert_eq!(clean("/a/..b/c.").unwrap(), "/a/..b/c.", "only whole `..` segments are refused");
    }

    fn r(inst: usize, path: &str, read_only: bool) -> Result<Resolved, Status> {
        Ok(Resolved { inst, path: path.into(), read_only })
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
    fn verbs_and_statuses_round_trip() {
        for (i, v) in VERBS.iter().enumerate() {
            assert_eq!(Verb::from_i32(i as i32), Some(*v));
            assert_eq!(Verb::from_keyword(v.keyword()), Some(*v));
        }
        assert_eq!(Verb::from_i32(8), None);
        assert_eq!(Status::from_i32(-1), None);
        assert!(Verb::Watch.is_safe() && !Verb::Remove.is_safe());
    }
}
