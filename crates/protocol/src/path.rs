//! Paths as requests and mounts write them.

use crate::status::Status;

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

/// A clean path to mount something at. Unlike a request path, a trailing
/// `/` means nothing here: `/www/` and `/www` are the same mount point.
pub fn mount_point(at: &str) -> Result<String, Status> {
    let mut at = clean(at)?;
    if at.len() > 1 && at.ends_with('/') {
        at.pop();
    }
    Ok(at)
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
        assert_eq!(mount_point("/a/b/").unwrap(), "/a/b");
        assert_eq!(mount_point("/").unwrap(), "/");
    }
}
