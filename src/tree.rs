//! Packed file trees: the files `include` copies into an image when it is
//! built, and the rules for copying them (see `docs/images.md`).

use crate::check::IncludeDecl;
use crate::pack::Hash;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Dir,
    File { hash: Hash, size: u64 },
}

/// Every path in the tree, clean and absolute without a trailing `/`, and
/// what is there. `/` is always a directory, and so is every parent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    nodes: BTreeMap<String, Node>,
}

impl Default for Tree {
    fn default() -> Tree {
        Tree { nodes: BTreeMap::from([("/".to_string(), Node::Dir)]) }
    }
}

impl Tree {
    pub fn get(&self, path: &str) -> Option<&Node> {
        self.nodes.get(path)
    }

    /// Every path and what is there, in order.
    pub fn nodes(&self) -> impl Iterator<Item = (&str, &Node)> {
        self.nodes.iter().map(|(p, n)| (p.as_str(), n))
    }

    /// The entries directly in directory `dir`, sorted by name.
    pub fn children<'t>(&'t self, dir: &str) -> impl Iterator<Item = (&'t str, &'t Node)> {
        let prefix = if dir == "/" { "/".to_string() } else { format!("{dir}/") };
        let skip = prefix.len();
        self.nodes.range(prefix.clone()..).take_while(move |(p, _)| p.starts_with(&prefix)).filter_map(move |(p, n)| {
            let name = &p[skip..];
            (!name.is_empty() && !name.contains('/')).then_some((name, n))
        })
    }

    /// The number of files and their total size.
    pub fn size(&self) -> (usize, u64) {
        self.nodes.values().fold((0, 0), |(n, total), node| match node {
            Node::File { size, .. } => (n + 1, total + size),
            Node::Dir => (n, total),
        })
    }

    /// Puts `node` at `path`, replacing whatever was there. A directory put
    /// on a directory merges with it. Missing parents are made, and a file
    /// in the way of one is replaced. Every path replaced is added to
    /// `replaced`.
    pub fn put(&mut self, path: &str, node: Node, replaced: &mut Vec<String>) {
        for (i, _) in path.match_indices('/').skip(1) {
            let parent = &path[..i];
            if let Some(Node::File { .. }) = self.nodes.insert(parent.to_string(), Node::Dir) {
                replaced.push(parent.to_string());
            }
        }
        match (self.nodes.get(path), &node) {
            (Some(Node::Dir), Node::Dir) => return,
            (Some(Node::Dir), Node::File { .. }) => {
                let under = format!("{path}/");
                self.nodes.retain(|p, _| !p.starts_with(&under));
                replaced.push(path.to_string());
            }
            (Some(Node::File { .. }), _) => replaced.push(path.to_string()),
            (None, _) => {}
        }
        self.nodes.insert(path.to_string(), node);
    }

    /// Applies one `include`, with sources relative to `dir`. `store` keeps a
    /// file's contents and names them. Returns the paths it replaced.
    pub fn include(
        &mut self,
        dir: &Path,
        inc: &IncludeDecl,
        store: &mut dyn FnMut(Vec<u8>) -> Hash,
    ) -> Result<Vec<String>> {
        let source = &inc.source;
        let mut replaced = vec![];
        let recursive = source.ends_with('/') || source.ends_with("/*");
        let real = dir.join(source.strip_suffix('*').unwrap_or(source).trim_end_matches('/'));
        let md = metadata(&real)?;
        if recursive {
            if !md.is_dir() {
                bail!("{} is not a directory", real.display());
            }
            let at = inc.at.as_deref().unwrap_or("/");
            self.walk(&real, at, &mut vec![], store, &mut replaced)?;
        } else if md.is_dir() {
            bail!("{} is a directory; write \"{}/\" to include everything under it", real.display(), source);
        } else {
            let at = match &inc.at {
                Some(at) => at.clone(),
                None => format!("/{}", file_name(&real)?),
            };
            if at == "/" {
                bail!("a single file can't be the root");
            }
            let node = self.file(&real, md.len(), store)?;
            self.put(&at, node, &mut replaced);
        }
        Ok(replaced)
    }

    /// Copies directory `real` to `at`, following symlinks. `stack` holds
    /// the directories being walked, to catch a link back to one of them.
    fn walk(
        &mut self,
        real: &Path,
        at: &str,
        stack: &mut Vec<PathBuf>,
        store: &mut dyn FnMut(Vec<u8>) -> Hash,
        replaced: &mut Vec<String>,
    ) -> Result<()> {
        let canonical = real.canonicalize().with_context(|| format!("reading {}", real.display()))?;
        if stack.contains(&canonical) {
            bail!("{} links back to {}, which contains it", real.display(), canonical.display());
        }
        stack.push(canonical);
        self.put(at, Node::Dir, replaced);
        let mut names = vec![];
        for entry in std::fs::read_dir(real).with_context(|| format!("reading {}", real.display()))? {
            names.push(entry.with_context(|| format!("reading {}", real.display()))?.file_name());
        }
        names.sort();
        for name in names {
            let child = real.join(&name);
            let name = file_name(&child)?;
            let child_at = if at == "/" { format!("/{name}") } else { format!("{at}/{name}") };
            let md = metadata(&child)?;
            if md.is_dir() {
                self.walk(&child, &child_at, stack, store, replaced)?;
            } else {
                let node = self.file(&child, md.len(), store)?;
                self.put(&child_at, node, replaced);
            }
        }
        stack.pop();
        Ok(())
    }

    fn file(&mut self, real: &Path, size: u64, store: &mut dyn FnMut(Vec<u8>) -> Hash) -> Result<Node> {
        let bytes = std::fs::read(real).with_context(|| format!("reading {}", real.display()))?;
        Ok(Node::File { hash: store(bytes), size })
    }
}

/// Like `std::fs::metadata`, which follows symlinks, but says when a link
/// leads nowhere and refuses anything that isn't a file or directory.
fn metadata(path: &Path) -> Result<std::fs::Metadata> {
    match std::fs::metadata(path) {
        Ok(md) if md.is_file() || md.is_dir() => Ok(md),
        Ok(_) => bail!("{} is neither a file nor a directory", path.display()),
        Err(_) if path.symlink_metadata().is_ok() => bail!("{} is a symlink to nothing", path.display()),
        Err(e) => Err(e).with_context(|| format!("cannot include {}", path.display())),
    }
}

fn file_name(path: &Path) -> Result<&str> {
    let name = path.file_name().with_context(|| format!("{} has no file name", path.display()))?;
    name.to_str().with_context(|| format!("{} is not valid UTF-8", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(n: u8) -> Node {
        Node::File { hash: n.to_string(), size: 1 }
    }

    fn paths(t: &Tree) -> Vec<&str> {
        t.nodes().map(|(p, _)| p).collect()
    }

    #[test]
    fn later_puts_win() {
        let mut t = Tree::default();
        let mut replaced = vec![];
        t.put("/a/b/c.txt", file(1), &mut replaced);
        t.put("/a/d.txt", file(2), &mut replaced);
        assert_eq!(paths(&t), ["/", "/a", "/a/b", "/a/b/c.txt", "/a/d.txt"], "parents are made");
        assert!(replaced.is_empty());

        t.put("/a", Node::Dir, &mut replaced);
        assert_eq!(t.get("/a/d.txt"), Some(&file(2)), "directories merge");
        t.put("/a/d.txt", file(3), &mut replaced);
        assert_eq!(t.get("/a/d.txt"), Some(&file(3)));
        t.put("/a/b", file(4), &mut replaced);
        assert_eq!(paths(&t), ["/", "/a", "/a/b", "/a/d.txt"], "a file replaces a whole directory");
        t.put("/a/d.txt/e", file(5), &mut replaced);
        assert_eq!(t.get("/a/d.txt"), Some(&Node::Dir), "and a directory replaces a file");
        assert_eq!(replaced, ["/a/d.txt", "/a/b", "/a/d.txt"]);
    }

    #[test]
    fn children_are_direct_and_sorted() {
        let mut t = Tree::default();
        for p in ["/b", "/a/x", "/a-z", "/a/y/z", "/c/"] {
            t.put(p.trim_end_matches('/'), file(0), &mut vec![]);
        }
        let names = |dir| t.children(dir).map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names("/"), ["a", "a-z", "b", "c"]);
        assert_eq!(names("/a"), ["x", "y"]);
        assert_eq!(names("/a/y"), ["z"]);
        assert!(names("/b").is_empty());
        assert_eq!(t.size(), (5, 5));
    }
}
