//! `Packed.Files`: files packed into the image when it was built, which it
//! answers for as `Files` does.

use super::Reply;
use super::files::{mime, name};
use protocol::{path::PathName, status::Status, verb::Verb};
use image::pack::Hash;
use protocol::payload::{Payload, Stat};
use image::tree::{Node, Tree};
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
    pub(super) fn serve(&self, req: u32, verb: Verb, path: &PathName) -> Vec<Reply> {
        if !verb.is_safe() {
            return vec![Reply::fail(req, Status::READ_ONLY)];
        }
        let as_dir = path.as_str().ends_with('/');
        let at = path.mount_point();
        let path = at.as_str();
        // As on disk, `file/` names nothing.
        let node = self.tree.get(path).filter(|n| !as_dir || **n == Node::Dir);
        let Some(node) = node else { return vec![Reply::fail(req, Status::NOT_FOUND)] };
        match (verb, node) {
            (Verb::Read, Node::Dir) => vec![Reply::fail(req, Status::IS_DIRECTORY)],
            (Verb::Read, Node::File { hash, .. }) => {
                vec![Reply::ok(req, Payload::new(mime(Path::new(path)), self.blobs[hash].to_vec()))]
            }
            (Verb::Stat, node) => vec![Reply::ok(req, stat(&name(path), node).encode())],
            (Verb::List, Node::File { .. }) => vec![Reply::fail(req, Status::NOT_DIRECTORY)],
            (Verb::List, Node::Dir) => {
                let mut replies: Vec<Reply> =
                    self.tree.children(path).map(|(name, node)| Reply::more(req, stat(name, node).encode())).collect();
                replies.push(Reply::end(req));
                replies
            }
            _ => vec![Reply::fail(req, Status::NOT_SUPPORTED)],
        }
    }
}

/// A packed file's stat record. Its version comes from its contents' hash,
/// so it changes exactly when they do.
fn stat(name: &str, node: &Node) -> Stat {
    match node {
        Node::Dir => Stat::directory(name),
        Node::File { hash, size } => {
            let version = hash.get(..16).and_then(|h| u64::from_str_radix(h, 16).ok()).unwrap_or(0);
            Stat { version, ..Stat::file(name, *size, mime(Path::new(name))) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> PathName {
        PathName::parse(text).unwrap()
    }

    fn status(replies: Vec<Reply>) -> Vec<Status> {
        replies.into_iter().map(|r| r.status).collect()
    }

    #[test]
    fn packed_files_resolve_paths_like_files() {
        let mut tree = Tree::default();
        tree.put("/a.txt", Node::File { hash: "h".into(), size: 1 }, &mut vec![]);
        tree.put("/sub", Node::Dir, &mut vec![]);
        let p = Packed { tree, blobs: BTreeMap::from([("h".to_string(), Rc::from(&b"A"[..]))]) };
        let read = |text: &str| status(p.serve(1, Verb::Read, &path(text)));
        assert_eq!(read("/a.txt"), [Status::OK]);
        assert_eq!(read("/./a.txt"), [Status::OK]);
        assert_eq!(read("/a.txt/"), [Status::NOT_FOUND]);
        assert_eq!(read("/sub/"), [Status::IS_DIRECTORY]);
        assert_eq!(status(p.serve(1, Verb::List, &path("/sub/"))), [Status::OK]);
        assert_eq!(status(p.serve(1, Verb::Watch, &path("/"))), [Status::NOT_SUPPORTED]);
        assert_eq!(status(p.serve(1, Verb::Create, &path("/b"))), [Status::READ_ONLY]);
        let stat = Stat::decode(&p.serve(1, Verb::Stat, &path("/a.txt")).remove(0).body.unwrap()).unwrap();
        assert_eq!((stat.name.as_str(), stat.length, stat.content_type.as_str()), ("a.txt", 1, "text/plain; charset=utf-8"));
    }
}
