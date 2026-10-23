//! A checked image, as the runtime runs it: every instance, what each one
//! sees, what it serves, the requests made at startup, and the slots its
//! host fills. See `docs/images.md`.

use crate::header::Object;
use anyhow::{Context, Result, bail};
use format::{AllSchemas, Formats};
use protocol::ns::{self, Resolved, Verb};
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
}

fn root() -> String {
    "/".into()
}

impl MountDecl {
    pub fn resolved(&self) -> Resolved {
        Resolved { inst: self.target, path: self.path.clone(), read_only: self.read_only }
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
            bail!("expected text, and it is {}", p.ty);
        }
        String::from_utf8(p.bytes).context("its text isn't UTF-8")
    }
}

/// `require <name> : <content type> [= <default>]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    /// The content type of what it stands for.
    pub ty: String,
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
    /// Whether its payloads are structured: one of `formats` encodes them
    /// from their fields.
    pub fn structured(&self, formats: &Formats) -> bool {
        formats.for_type(&self.ty).is_some()
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
    // Mounting an image's instance reaches what it exports, and nothing
    // else of its view, so reach goes through its exports.
    let mounts: Vec<Vec<(usize, bool)>> = p
        .instances
        .iter()
        .map(|i| {
            let edges = match &i.kind {
                Kind::Image { exports } => exports.as_slice(),
                _ => &i.mounts,
            };
            edges.iter().map(|m| (m.target, m.read_only)).collect()
        })
        .collect();
    let reach = ns::reach(&mounts);
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

