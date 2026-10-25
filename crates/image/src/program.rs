//! A checked image, as the runtime runs it: every instance, what each one
//! sees, what it serves, the requests made at startup, and the slots its
//! host fills. See `docs/images.md`.

use crate::header::Object;
use anyhow::{Context, Result, bail};
use format::AllSchemas;
use protocol::{namespace::Resolved, verb::Verb};
use protocol::payload::Payload;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::PathBuf;
use syntax::image_definition::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// Each format's schemas, by its content type: every type the image's
    /// modules declare, which its literals and slots are encoded with.
    pub messages: AllSchemas,
    /// Each instance type's interface: a module's header, the runtime's,
    /// or for an instance of an image, what it needs and exports.
    pub interfaces: BTreeMap<String, Object>,
    /// Values whoever runs the image provides, in image order; [`Arg::Slot`]
    /// refers to them by index. Only the root image's slots are theirs.
    pub slots: Vec<Slot>,
    /// Every instance, each image's in its order, an instantiated image's
    /// own after its image's, and everything else refers to them by index. An
    /// instance of an image is one too, whose view is its image's and which
    /// serves what the image exports. The root image's needs come first.
    pub instances: Vec<Instance>,
    /// The root image's own view, which its startup requests resolve in.
    pub view: Vec<MountDecl>,
    /// Every image's startup requests, in the order they are made: an
    /// instantiated image's before those of the image instantiating it.
    pub startup: Vec<Startup>,
    /// The `.oth` files the image was checked from: its own, those of the
    /// images it instantiates, and the headers they import.
    /// Not kept in a built image, which has copies under its sources.
    #[serde(skip)]
    pub sources: Vec<PathBuf>,
}

impl Program {
    /// The order instances start in: each after everything it mounts, and
    /// an image's instance after everything it exports, but otherwise in
    /// image order. Where instances mount each other, the cycle is broken at
    /// the one that comes first in the image, which starts last.
    pub fn start_order(&self) -> Vec<usize> {
        let mounts: Vec<Vec<usize>> = self
            .instances
            .iter()
            .map(|instance| {
                let exports = match &instance.kind {
                    Kind::Image { exports } => exports.as_slice(),
                    _ => &[],
                };
                instance.mounts.iter().chain(exports).map(|mount| mount.target).collect()
            })
            .collect();
        start_order(&mounts)
    }

    /// For each instance, every other instance it could ever reach, given
    /// the image's mounts.
    ///
    /// Views and exposures only ever hold names their owner could already
    /// resolve, so this transitive closure bounds everything bind and expose
    /// can do at run time. A target counts as read-write if any path to it
    /// is. Mounting an image's instance reaches what it exports, and nothing
    /// else of its view, so reach goes through its exports.
    pub fn reach(&self) -> Vec<BTreeMap<usize, Reach>> {
        let mounts: Vec<Vec<(usize, bool)>> = self
            .instances
            .iter()
            .map(|instance| {
                let edges = match &instance.kind {
                    Kind::Image { exports } => exports.as_slice(),
                    _ => &instance.mounts,
                };
                edges.iter().map(|mount| (mount.target, mount.read_only)).collect()
            })
            .collect();
        reach(&mounts)
    }
}

/// How an instance may reach another one, at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    pub read_only: bool,
    /// The instance mounted in its own view that the reach goes through;
    /// `None` when the target itself is mounted there.
    pub via: Option<usize>,
}

/// [`Program::reach`], where `mounts[i]` lists `(target, read_only)` for
/// each of instance `i`'s mounts.
fn reach(mounts: &[Vec<(usize, bool)>]) -> Vec<BTreeMap<usize, Reach>> {
    (0..mounts.len())
        .map(|start| {
            let mut reached: BTreeMap<usize, Reach> = BTreeMap::new();
            // (instance, read-only so far, the mount in start's view this goes through)
            let mut work: Vec<(usize, bool, Option<usize>)> = vec![(start, false, None)];
            let mut seen = vec![[false; 2]; mounts.len()];
            while let Some((at, read_only, first)) = work.pop() {
                for &(target, mount_read_only) in &mounts[at] {
                    let read_only = read_only || mount_read_only;
                    let first = first.unwrap_or(target);
                    if std::mem::replace(&mut seen[target][read_only as usize], true) {
                        continue;
                    }
                    if target != start {
                        let reach = Reach { read_only, via: (first != target).then_some(first) };
                        match reached.get_mut(&target) {
                            Some(existing) if existing.read_only && !read_only => *existing = reach,
                            Some(_) => {}
                            None => {
                                reached.insert(target, reach);
                            }
                        }
                    }
                    work.push((target, read_only, Some(first)));
                }
            }
            reached
        })
        .collect()
}

/// [`Program::start_order`], where `mounts[i]` lists the targets of each of
/// instance `i`'s mounts.
fn start_order(mounts: &[Vec<usize>]) -> Vec<usize> {
    fn visit(at: usize, mounts: &[Vec<usize>], seen: &mut [bool], order: &mut Vec<usize>) {
        if std::mem::replace(&mut seen[at], true) {
            return;
        }
        for &target in &mounts[at] {
            visit(target, mounts, seen, order);
        }
        order.push(at);
    }
    let mut seen = vec![false; mounts.len()];
    let mut order = Vec::with_capacity(mounts.len());
    for instance in 0..mounts.len() {
        visit(instance, mounts, &mut seen, &mut order);
    }
    order
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    /// Under the names of the image instances it is in: `web.http`. One of
    /// the root image's needs is named by its mount point, `/net/tcp`.
    pub name: String,
    /// The key of its interface in [`Program::interfaces`].
    pub type_name: String,
    pub kind: Kind,
    /// The directory of the image that declared it, relative to the root's,
    /// which its relative paths (includes, a files root) start from.
    pub dir: String,
    /// An environment object's config.
    pub config: Option<Arg>,
    /// The initial view.
    pub mounts: Vec<MountDecl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MountDecl {
    /// Clean, absolute.
    pub at: String,
    pub target: usize,
    /// The path in the target's served tree that is put there: `/` for
    /// all of it, or another for a bind.
    #[serde(default = "root")]
    pub path: String,
    pub read_only: bool,
    /// Made by a `bind`: what something else already holds, passed on.
    /// Otherwise it is a `mount`, and the target is held here.
    #[serde(default)]
    pub bound: bool,
}

fn root() -> String {
    "/".into()
}

impl MountDecl {
    pub fn resolved(&self) -> Resolved {
        Resolved { instance: self.target, path: self.path.clone(), read_only: self.read_only }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Startup {
    /// The instance of an image whose view it resolves in, or `None` for
    /// the root image's.
    pub image: Option<usize>,
    pub verb: Verb,
    /// Clean, in the image's view.
    pub path: String,
    pub body: Option<Arg>,
}

/// What the image gives in a startup request: a payload, or one of the
/// root image's slots standing for one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Arg {
    Payload(Payload),
    Slot(usize),
}

impl Arg {
    /// What it carries, once any slot has been bound (see `slots::bind`).
    pub fn payload(&self) -> Result<Payload> {
        match self {
            Arg::Payload(p) => Ok(p.clone()),
            Arg::Slot(i) => bail!("slot {i} has no value; the image hasn't been bound"),
        }
    }

    /// Its text, for a text payload, once any slot has been bound.
    pub fn text(&self) -> Result<String> {
        let p = self.payload()?;
        if !p.is_text() {
            bail!("expected text, and it is {}", p.content_type);
        }
        String::from_utf8(p.bytes).context("its text isn't UTF-8")
    }
}

/// `require <name> : <message type or content type> [= <default>]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    /// The content type of what it stands for.
    pub ty: String,
    /// Its message type, for a structured slot, whose value is given field
    /// by field.
    #[serde(default)]
    pub message: Option<String>,
    /// The image's default: text, or a structured value, which may leave
    /// some fields out.
    pub default: Option<SlotValue>,
    /// Once bound: the value, and where it, or each of its fields, came from.
    #[serde(skip)]
    pub value: Option<Arg>,
    #[serde(skip)]
    pub origins: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SlotValue {
    Text(String),
    Tree(Value),
}

impl Slot {
    /// Whether its payloads are structured: messages, which its format
    /// encodes from their fields.
    pub fn structured(&self) -> bool {
        self.message.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    /// A module: its object's name, and where the module is, relative to
    /// the root image's directory.
    Wasm { type_name: String, module: String },
    Console,
    /// Listens on the address its config gives.
    Tcp,
    /// Serves files under the directory its config gives. A relative path
    /// is relative to the image.
    Files,
    /// Serves the files its includes packed into the image, in order.
    Packed { includes: Vec<IncludeDecl> },
    /// A console in memory: Console and Terminal.
    MemoryConsole,
    /// A listener in memory: Listener and Dialer.
    MemoryListener,
    /// An instance of an image: its view is the instance's mounts, and it
    /// serves what these put in its served tree, from that view.
    Image { exports: Vec<MountDecl> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncludeDecl {
    /// As written, relative to the image.
    pub source: String,
    /// Clean and absolute, without a trailing `/`.
    pub at: Option<String>,
}

/// A summary of each instance's view and the most it can ever reach, for
/// `--check`. `about` may say more about an instance, on a line of its own;
/// `skip` leaves one out. Instances of images aren't shown, only what is
/// in them.
pub fn describe(p: &Program, about: &dyn Fn(usize) -> Option<String>, skip: &dyn Fn(usize) -> bool) -> String {
    let reach = p.reach();
    let shown = |i: usize| !skip(i) && !matches!(p.instances[i].kind, Kind::Image { .. });
    let width = (0..p.instances.len()).filter(|&i| shown(i)).map(|i| p.instances[i].name.len()).max().unwrap_or(0);
    let target = |m: &MountDecl| {
        let name = &p.instances[m.target].name;
        if m.path == "/" { name.clone() } else { format!("{name}{}", m.path) }
    };
    let mut out = String::new();
    for (i, inst) in p.instances.iter().enumerate().filter(|(i, _)| shown(*i)) {
        let mut name = inst.name.as_str();
        if let Some(about) = about(i) {
            writeln!(out, "{name:width$}  {about}").unwrap();
            name = "";
        }
        if inst.mounts.is_empty() {
            if !name.is_empty() {
                writeln!(out, "{name:width$}  sees nothing").unwrap();
            }
            continue;
        }
        let sees: Vec<String> =
            inst.mounts.iter().map(|m| format!("{} → {}{}", m.at, target(m), if m.read_only { " ro" } else { "" })).collect();
        writeln!(out, "{name:width$}  sees {}", sees.join(", ")).unwrap();
        let reaches: Vec<String> = reach[i]
            .iter()
            .filter(|(t, _)| !matches!(p.instances[**t].kind, Kind::Image { .. }))
            .map(|(&t, r)| {
                let mode = if r.read_only { "ro" } else { "rw" };
                match r.via {
                    Some(v) => format!("{} {mode} (via {})", p.instances[t].name, p.instances[v].name),
                    None => format!("{} {mode}", p.instances[t].name),
                }
            })
            .collect();
        writeln!(out, "{:width$}  can reach at most: {}", "", reaches.join(", ")).unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reach_is_the_closure_of_mounts() {
        // http(0) mounts net(1) and site(2); site mounts files(3) read-only.
        let mounts = vec![vec![(1, false), (2, false)], vec![], vec![(3, true)], vec![]];
        let reach = reach(&mounts);
        let reached = |from: usize, target: usize| reach[from].get(&target).map(|r| (r.read_only, r.via));
        assert_eq!(reached(0, 1), Some((false, None)));
        assert_eq!(reached(0, 2), Some((false, None)));
        assert_eq!(reached(0, 3), Some((true, Some(2))));
        assert_eq!(reached(2, 3), Some((true, None)));
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
