//! `Environment.Files`: read-only access to one directory on the host.

use super::{Reply, msg};
use crate::ns::{Status, Verb};
use crate::wire;
use anyhow::{Context, Result, bail};
use bson::doc;
use std::path::{Component, Path, PathBuf};

pub struct Files {
    /// Canonical, so symlinks can be checked against it.
    root: PathBuf,
}

impl Files {
    /// Serves the directory at `root`, which must exist.
    pub(super) fn open(root: PathBuf) -> Result<Files> {
        let root = root.canonicalize().with_context(|| format!("files root {}", root.display()))?;
        if !root.is_dir() {
            bail!("files root {} is not a directory", root.display());
        }
        Ok(Files { root })
    }

    pub(super) fn serve(&self, req: u32, verb: Verb, path: &str) -> Vec<Reply> {
        if !verb.is_safe() {
            return vec![Reply::fail(req, Status::ReadOnly)];
        }
        let real = match self.locate(path) {
            Ok(p) => p,
            Err(status) => return vec![Reply::fail(req, status)],
        };
        let io = |e: std::io::Error| if e.kind() == std::io::ErrorKind::NotFound { Status::NotFound } else { Status::Io };
        match verb {
            Verb::Read if real.is_dir() => vec![Reply::fail(req, Status::IsDirectory)],
            Verb::Read => match std::fs::read(&real) {
                Ok(data) => vec![Reply::ok(req, msg("Data", doc! { "mime": mime(&real), "data": wire::bytes(data) }))],
                Err(e) => vec![Reply::fail(req, io(e))],
            },
            Verb::Stat => match real.metadata() {
                Ok(md) => {
                    let (kind, mime) = if md.is_dir() { ("dir", "") } else { ("file", mime(&real)) };
                    vec![Reply::ok(req, msg("Info", doc! { "kind": kind, "size": md.len() as i64, "mime": mime }))]
                }
                Err(e) => vec![Reply::fail(req, io(e))],
            },
            Verb::List if !real.is_dir() => vec![Reply::fail(req, Status::NotDirectory)],
            Verb::List => match std::fs::read_dir(&real) {
                Ok(entries) => {
                    let mut names: Vec<(String, &str)> = entries
                        .flatten()
                        .map(|e| {
                            let kind = if e.path().is_dir() { "dir" } else { "file" };
                            (e.file_name().to_string_lossy().into_owned(), kind)
                        })
                        .collect();
                    names.sort();
                    let mut replies: Vec<Reply> = names
                        .into_iter()
                        .map(|(name, kind)| Reply::more(req, msg("Entry", doc! { "name": name, "kind": kind })))
                        .collect();
                    replies.push(Reply::end(req));
                    replies
                }
                Err(e) => vec![Reply::fail(req, io(e))],
            },
            _ => vec![Reply::fail(req, Status::NotSupported)],
        }
    }

    /// Finds `path` under the root. Never leaves it: `..` is refused and
    /// symlinks are resolved and re-checked. Anything outside the root is
    /// `NotFound`, as if it didn't exist.
    fn locate(&self, path: &str) -> std::result::Result<PathBuf, Status> {
        let full = resolve(&self.root, path)?;
        let real = full.canonicalize().map_err(|_| Status::NotFound)?;
        if !real.starts_with(&self.root) {
            return Err(Status::NotFound);
        }
        Ok(real)
    }
}

fn resolve(root: &Path, path: &str) -> std::result::Result<PathBuf, Status> {
    if path.contains('\0') || path.contains('\\') {
        return Err(Status::Invalid);
    }
    let mut out = root.to_path_buf();
    for c in Path::new(path.trim_start_matches('/')).components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return Err(Status::Invalid),
        }
    }
    Ok(out)
}

/// A MIME type from the file extension.
pub(super) fn mime(path: &Path) -> &'static str {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "txt" | "md" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(replies: Vec<Reply>) -> Vec<Status> {
        replies.into_iter().map(|r| r.status).collect()
    }

    #[test]
    fn files_stay_inside_the_root() {
        let base = std::env::temp_dir().join(format!("othismo_files_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("root/sub")).unwrap();
        std::fs::write(base.join("root/a.txt"), "A").unwrap();
        std::fs::write(base.join("root/sub/b.css"), "B").unwrap();
        std::fs::write(base.join("secret.txt"), "S").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("secret.txt"), base.join("root/link.txt")).unwrap();

        let f = Files { root: base.join("root").canonicalize().unwrap() };
        let read = |p: &str| f.serve(1, Verb::Read, p);
        let a = read("/a.txt").remove(0).body.unwrap();
        assert_eq!(a.fields.get_binary_generic("data").unwrap(), b"A");
        assert_eq!(a.fields.get_str("mime").unwrap(), "text/plain; charset=utf-8");
        assert_eq!(status(read("sub/./b.css")), [Status::Ok]);
        assert_eq!(status(read("/sub")), [Status::IsDirectory]);
        assert_eq!(status(read("/missing")), [Status::NotFound]);
        assert_eq!(status(read("/../secret.txt")), [Status::Invalid]);
        assert_eq!(status(read("sub\\..\\a.txt")), [Status::Invalid]);
        #[cfg(unix)]
        assert_eq!(status(read("/link.txt")), [Status::NotFound], "symlinks out of the root don't exist");
        assert_eq!(status(f.serve(1, Verb::Write, "/a.txt")), [Status::ReadOnly]);

        let info = f.serve(1, Verb::Stat, "/sub/b.css").remove(0).body.unwrap();
        assert_eq!(info.fields, doc! { "kind": "file", "size": 1i64, "mime": "text/css; charset=utf-8" });
        let list: Vec<(String, bool)> = f
            .serve(1, Verb::List, "/")
            .into_iter()
            .map(|r| (r.body.map(|b| b.fields.get_str("name").unwrap().to_string()).unwrap_or_default(), r.done))
            .collect();
        let mut want = vec![("a.txt".to_string(), false), ("link.txt".into(), false), ("sub".into(), false), ("".into(), true)];
        if cfg!(not(unix)) {
            want.remove(1);
        }
        assert_eq!(list, want);
        assert_eq!(status(f.serve(1, Verb::List, "/a.txt")), [Status::NotDirectory]);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
