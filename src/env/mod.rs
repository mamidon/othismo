//! Environmental objects: the only things in an image that touch the outside
//! world. Each is a server the runtime implements, configured by the image
//! alone, serving requests made of its tree. Their interfaces are the
//! runtime's own header, `env`, one file per object in `as/env/`, and their
//! implementations are here, one module per object.
//!
//! Objects that wait on the outside world (stdin, sockets) do the waiting on
//! their own threads and report [`Event`]s over a channel. All requests and
//! replies still happen on the runtime's single thread.

mod console;
mod files;
mod packed;
mod tcp;

pub use console::{Console, MemoryConsole, spawn_stdin};
pub use files::Files;
pub use packed::Packed;
pub use tcp::{MemoryListener, Tcp};

use crate::check::{Instance, IncludeDecl, Kind};
use crate::messages::MessageType;
use crate::ns::{Status, Verb};
use crate::pack::{self, Pack};
use crate::tree::Tree;
use crate::wire::Message;
use anyhow::{Context, Result};
use bson::Document;
use std::collections::BTreeMap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::mpsc::Sender;

/// Something happened outside; `to` is the instance it concerns.
pub struct Event {
    pub to: usize,
    pub what: What,
}

pub enum What {
    Line(String),
    Eof,
    Accepted { conn: u32, stream: TcpStream },
    Data { conn: u32, data: Vec<u8> },
    Closed { conn: u32 },
}

/// A reply to request `req`, which this object is serving.
#[derive(Debug, PartialEq)]
pub struct Reply {
    pub req: u32,
    pub status: Status,
    pub body: Option<Message>,
    /// The last reply to `req`.
    pub done: bool,
}

impl Reply {
    fn ok(req: u32, body: Message) -> Reply {
        Reply { req, status: Status::Ok, body: Some(body), done: true }
    }

    fn more(req: u32, body: Message) -> Reply {
        Reply { req, status: Status::Ok, body: Some(body), done: false }
    }

    fn end(req: u32) -> Reply {
        Reply { req, status: Status::Ok, body: None, done: true }
    }

    fn fail(req: u32, status: Status) -> Reply {
        Reply { req, status, body: None, done: true }
    }
}

pub enum EnvObject {
    Console(Console),
    Tcp(Tcp),
    Files(Files),
    Packed(Packed),
    /// A Console in an imported image.
    MemoryConsole(MemoryConsole),
    /// A Listener in an imported image.
    MemoryListener(MemoryListener),
}

fn msg(type_name: &str, fields: Document) -> Message {
    Message { type_name: type_name.into(), fields }
}

impl EnvObject {
    /// `None` for instances implemented in Wasm. A Tcp listener is bound
    /// here, so its address is known before anything starts. A relative
    /// files root is relative to the directory of the image that declared
    /// it. One of the runtime's objects in an imported image runs in memory:
    /// a Files instance serves a snapshot of its directory, taken now.
    pub fn new(inst: &Instance, pack: &Pack) -> Result<Option<EnvObject>> {
        let dir = pack.dir.join(&inst.dir);
        Ok(Some(match &inst.kind {
            Kind::Wasm { .. } => return Ok(None),
            Kind::Console if inst.in_memory => EnvObject::MemoryConsole(MemoryConsole::default()),
            Kind::Console => EnvObject::Console(Console::default()),
            Kind::Tcp if inst.in_memory => EnvObject::MemoryListener(MemoryListener::default()),
            Kind::Tcp => EnvObject::Tcp(Tcp::bind(&config_str(inst, "address")?)?),
            Kind::Files if inst.in_memory => {
                let root = config_str(inst, "path")?;
                let snapshot = IncludeDecl { source: format!("{}/", root.trim_end_matches('/')), at: None };
                let (mut tree, mut blobs) = (Tree::default(), BTreeMap::new());
                tree.include(&dir, &snapshot, &mut |bytes| pack::add(&mut blobs, bytes))
                    .with_context(|| format!("taking a snapshot of {}", dir.join(&root).display()))?;
                EnvObject::Packed(Packed::new(tree, blobs))
            }
            Kind::Files => EnvObject::Files(Files::open(dir.join(config_str(inst, "path")?))?),
            Kind::Packed { .. } => {
                let tree = pack.trees.get(&inst.name).context("its files are missing")?.clone();
                EnvObject::Packed(Packed::new(tree, pack.blobs.clone()))
            }
        }))
    }

    /// Starts listening for outside events. Called once the image is set up.
    pub fn start(&mut self, me: usize, events: &Sender<Event>) {
        if let EnvObject::Tcp(t) = self {
            t.start(me, events);
        }
    }

    /// Serves request `req`. `out` and `messages` are for printing.
    pub fn serve(
        &mut self,
        req: u32,
        verb: Verb,
        path: &str,
        body: Option<&Message>,
        out: &mut dyn Write,
        messages: &BTreeMap<String, MessageType>,
    ) -> Result<Vec<Reply>> {
        // Their config is taken from the image's `control /` before they are
        // made; changing it while they run isn't supported yet.
        if verb == Verb::Control {
            return Ok(vec![Reply::fail(req, Status::NotSupported)]);
        }
        Ok(match self {
            EnvObject::Console(c) => c.serve(req, verb, path, body, out, messages)?,
            EnvObject::Tcp(t) => t.serve(req, verb, path, body),
            EnvObject::Files(f) => f.serve(req, verb, path),
            EnvObject::Packed(p) => p.serve(req, verb, path),
            EnvObject::MemoryConsole(c) => c.serve(req, verb, path, body),
            EnvObject::MemoryListener(l) => l.serve(req, verb, path, body),
        })
    }

    /// The caller gave up on `req`.
    pub fn cancel(&mut self, req: u32) -> Vec<Reply> {
        let mut replies = match self {
            EnvObject::Console(c) => {
                c.cancel(req);
                vec![]
            }
            EnvObject::Tcp(t) => {
                t.cancel(req);
                vec![]
            }
            EnvObject::MemoryConsole(c) => {
                c.cancel(req);
                vec![]
            }
            EnvObject::MemoryListener(l) => l.cancel(req),
            EnvObject::Files(_) | EnvObject::Packed(_) => vec![],
        };
        replies.push(Reply::end(req));
        replies
    }

    /// Turns an outside event into replies, updating state on the way.
    pub fn on_event(&mut self, what: What) -> Vec<Reply> {
        match self {
            EnvObject::Console(c) => c.on_event(what),
            EnvObject::Tcp(t) => t.on_event(what),
            _ => unreachable!("only stdin and sockets report events"),
        }
    }
}

/// A string field of an environment object's config.
fn config_str(inst: &Instance, field: &str) -> Result<String> {
    let config = inst.config.as_ref().context("no config")?.message()?;
    Ok(config.fields.get_str(field).with_context(|| format!("config has no `{field}`"))?.to_string())
}
