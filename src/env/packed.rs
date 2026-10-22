//! `Packed.Files`: files packed into the image when it was built, which it
//! answers for as `Files` does.

use super::files::mime;
use super::{Reply, msg};
use crate::ns::{self, Status, Verb};
use crate::pack::Hash;
use crate::tree::{Node, Tree};
use crate::wire;
use bson::doc;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

pub struct Packed {
    tree: Tree,
    blobs: BTreeMap<Hash, Rc<[u8]>>,
}

impl Packed {
    pub(super) fn new(tree: Tree, blobs: BTreeMap<Hash, Rc<[u8]>>) -> Packed {
        Packed { tree, blobs }
    }

    /// Answers as [`Files`] does, from the tree.
    pub(super) fn serve(&self, req: u32, verb: Verb, path: &str) -> Vec<Reply> {
        if !verb.is_safe() {
            return vec![Reply::fail(req, Status::ReadOnly)];
        }
        let as_dir = path.len() > 1 && path.ends_with('/');
        let Ok(path) = ns::mount_point(path) else { return vec![Reply::fail(req, Status::Invalid)] };
        // As on disk, `file/` names nothing.
        let node = self.tree.get(&path).filter(|n| !as_dir || **n == Node::Dir);
        let Some(node) = node else { return vec![Reply::fail(req, Status::NotFound)] };
        let mime = || mime(Path::new(&path));
        match (verb, node) {
            (Verb::Read, Node::Dir) => vec![Reply::fail(req, Status::IsDirectory)],
            (Verb::Read, Node::File { hash, .. }) => {
                let data = wire::bytes(self.blobs[hash].to_vec());
                vec![Reply::ok(req, msg("Data", doc! { "mime": mime(), "data": data }))]
            }
            (Verb::Stat, Node::Dir) => vec![Reply::ok(req, msg("Info", doc! { "kind": "dir", "size": 0i64, "mime": "" }))],
            (Verb::Stat, Node::File { size, .. }) => {
                vec![Reply::ok(req, msg("Info", doc! { "kind": "file", "size": *size as i64, "mime": mime() }))]
            }
            (Verb::List, Node::File { .. }) => vec![Reply::fail(req, Status::NotDirectory)],
            (Verb::List, Node::Dir) => {
                let mut replies: Vec<Reply> = self
                    .tree
                    .children(&path)
                    .map(|(name, node)| {
                        let kind = if *node == Node::Dir { "dir" } else { "file" };
                        Reply::more(req, msg("Entry", doc! { "name": name, "kind": kind }))
                    })
                    .collect();
                replies.push(Reply::end(req));
                replies
            }
            _ => vec![Reply::fail(req, Status::NotSupported)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(replies: Vec<Reply>) -> Vec<Status> {
        replies.into_iter().map(|r| r.status).collect()
    }

    #[test]
    fn packed_files_resolve_paths_like_files() {
        let mut tree = Tree::default();
        tree.put("/a.txt", Node::File { hash: "h".into(), size: 1 }, &mut vec![]);
        tree.put("/sub", Node::Dir, &mut vec![]);
        let p = Packed { tree, blobs: BTreeMap::from([("h".to_string(), Rc::from(&b"A"[..]))]) };
        let read = |path: &str| status(p.serve(1, Verb::Read, path));
        assert_eq!(read("/a.txt"), [Status::Ok]);
        assert_eq!(read("/./a.txt"), [Status::Ok]);
        assert_eq!(read("/a.txt/"), [Status::NotFound]);
        assert_eq!(read("/sub/"), [Status::IsDirectory]);
        assert_eq!(read("/sub/../a.txt"), [Status::Invalid]);
        assert_eq!(status(p.serve(1, Verb::List, "/sub/")), [Status::Ok]);
        assert_eq!(status(p.serve(1, Verb::Watch, "/")), [Status::NotSupported]);
        assert_eq!(status(p.serve(1, Verb::Create, "/b")), [Status::ReadOnly]);
    }
}
