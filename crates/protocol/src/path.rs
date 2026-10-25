//! Paths as requests and mounts write them.

use crate::status::Status;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

/// A clean absolute path: no `//`, `.` or `..` segments, and no NUL bytes.
/// A trailing `/` is kept, since a request means something by it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PathName(String);

impl PathName {
    pub fn root() -> PathName {
        PathName("/".into())
    }

    /// Cleans an absolute path: collapses `//` and `.` segments and keeps a
    /// trailing `/`. Relative paths, `..` segments and NUL bytes are `INVALID`.
    pub fn parse(text: &str) -> Result<PathName, Status> {
        if !text.starts_with('/') || text.contains('\0') {
            return Err(Status::INVALID);
        }
        let mut cleaned = String::with_capacity(text.len());
        for segment in text.split('/') {
            match segment {
                "" | "." => {}
                ".." => return Err(Status::INVALID),
                segment => {
                    cleaned.push('/');
                    cleaned.push_str(segment);
                }
            }
        }
        if cleaned.is_empty() || text.ends_with('/') || text.ends_with("/.") {
            cleaned.push('/');
        }
        Ok(PathName(cleaned))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The mount point this path names, which drops a trailing `/`.
    pub fn mount_point(&self) -> MountPoint {
        match self.0.strip_suffix('/') {
            Some(trimmed) if !trimmed.is_empty() => MountPoint(PathName(trimmed.into())),
            _ => MountPoint(self.clone()),
        }
    }

    /// Each mount point that could prefix this path on a segment boundary,
    /// longest first and `/` last, with the rest of the path below it: `""`
    /// for the mount point itself, or else starting with `/`.
    pub(crate) fn mount_points(&self) -> impl Iterator<Item = (&str, &str)> {
        let path = self.as_str();
        // Each prefix that ends before a `/`, longest first, past any trailing
        // `/`; `end == 0` stands for `/` itself.
        let mut end = Some(path.strip_suffix('/').unwrap_or(path).len());
        std::iter::from_fn(move || {
            let current = end?;
            end = (current > 0).then(|| path[..current].rfind('/').expect("a clean path starts with `/`"));
            Some(match current {
                0 if path == "/" => ("/", ""),
                0 => ("/", path),
                current => (&path[..current], &path[current..]),
            })
        })
    }

    /// What `rest`, the rest of a path below a mount point as
    /// [`mount_points`](PathName::mount_points) gives it, names below this.
    pub(crate) fn join(&self, rest: &str) -> PathName {
        match (self.as_str(), rest) {
            (_, "") => self.clone(),
            ("/", rest) => PathName(rest.into()),
            (base, rest) => PathName(format!("{base}{rest}")),
        }
    }
}

/// A clean path to mount something at. Unlike a request's path, a trailing
/// `/` means nothing here: `/www/` and `/www` are the same mount point.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MountPoint(PathName);

impl MountPoint {
    pub fn root() -> MountPoint {
        MountPoint(PathName::root())
    }

    pub fn parse(text: &str) -> Result<MountPoint, Status> {
        Ok(PathName::parse(text)?.mount_point())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The mount point as a path, as a request would name it.
    pub fn path(&self) -> &PathName {
        &self.0
    }
}

// Mount tables are looked up by the prefixes of a path.
impl Borrow<str> for MountPoint {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for PathName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for MountPoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for PathName {
    type Error = String;
    fn try_from(text: String) -> Result<PathName, String> {
        PathName::parse(&text).map_err(|_| format!("`{text}` is not a clean absolute path"))
    }
}

impl TryFrom<String> for MountPoint {
    type Error = String;
    fn try_from(text: String) -> Result<MountPoint, String> {
        MountPoint::parse(&text).map_err(|_| format!("`{text}` is not a clean absolute path"))
    }
}

impl From<PathName> for String {
    fn from(path: PathName) -> String {
        path.0
    }
}

impl From<MountPoint> for String {
    fn from(at: MountPoint) -> String {
        at.0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(text: &str) -> Result<String, Status> {
        PathName::parse(text).map(String::from)
    }

    #[test]
    fn cleaning_paths() {
        assert_eq!(clean("/").unwrap(), "/");
        assert_eq!(clean("//a//b/./c").unwrap(), "/a/b/c");
        assert_eq!(clean("/a/b/").unwrap(), "/a/b/");
        assert_eq!(clean("/a/.").unwrap(), "/a/");
        assert_eq!(clean("/."), Ok("/".into()));
        assert_eq!(clean("a/b"), Err(Status::INVALID));
        assert_eq!(clean(""), Err(Status::INVALID));
        assert_eq!(clean("/a/../b"), Err(Status::INVALID));
        assert_eq!(clean("/a/.."), Err(Status::INVALID));
        assert_eq!(clean("/a\0"), Err(Status::INVALID));
        assert_eq!(clean("/a/..b/c.").unwrap(), "/a/..b/c.", "only whole `..` segments are refused");
        assert_eq!(MountPoint::parse("/a/b/").unwrap().as_str(), "/a/b");
        assert_eq!(MountPoint::parse("/").unwrap(), MountPoint::root());
        assert_eq!(MountPoint::parse("relative"), Err(Status::INVALID));
    }

    #[test]
    fn mount_points_longest_first() {
        let mount_points = |text| PathName::parse(text).unwrap().mount_points().map(|(at, rest)| format!("{at} {rest}")).collect::<Vec<_>>();
        assert_eq!(mount_points("/a/b"), ["/a/b ", "/a /b", "/ /a/b"]);
        assert_eq!(mount_points("/a/"), ["/a /", "/ /a/"]);
        assert_eq!(mount_points("/"), ["/ "]);
    }

    #[test]
    fn paths_load_only_if_clean() {
        let load = |text: &str| PathName::try_from(text.to_string());
        assert_eq!(load("/a//b").map(String::from), Ok("/a/b".into()));
        assert_eq!(load("a").unwrap_err(), "`a` is not a clean absolute path");
    }
}
