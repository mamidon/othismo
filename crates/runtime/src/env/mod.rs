//! Environmental objects: the only things in an image that touch the outside
//! world. Each is a server the runtime implements, configured by the image
//! alone, serving requests made of its tree. Their interfaces are the
//! runtime's own header, `env`, one file per object in `sdk/env/`, and their
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

use image::program::{Instance, Kind};
use protocol::ns::{Status, Verb};
use image::pack::Pack;
use protocol::payload::Payload;
use anyhow::{Context, Result};
use std::io::Write;
use std::net::TcpStream;
use std::sync::mpsc::Sender;

/// Something happened outside; `to` is the instance it concerns.
pub struct Event {
    pub to: usize,
    pub what: What,
}

/// What the runtime's loop waits for: an event from outside, or a command
/// from the local operator (see [`crate::operator`]).
pub enum Incoming {
    Event(Event),
    Command(crate::operator::Command),
}

impl From<Event> for Incoming {
    fn from(e: Event) -> Incoming {
        Incoming::Event(e)
    }
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
    pub body: Option<Payload>,
    /// The last reply to `req`.
    pub done: bool,
}

impl Reply {
    fn ok(req: u32, body: Payload) -> Reply {
        Reply { req, status: Status::Ok, body: Some(body), done: true }
    }

    fn more(req: u32, body: Payload) -> Reply {
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
    /// A console in memory.
    MemoryConsole(MemoryConsole),
    /// A listener in memory.
    MemoryListener(MemoryListener),
}

impl EnvObject {
    /// `None` for instances implemented in Wasm and instances of images. A
    /// Tcp listener is bound here, so its address is known before anything
    /// starts. A relative files root is relative to the directory of the
    /// image that declared it.
    pub fn new(inst: &Instance, pack: &Pack) -> Result<Option<EnvObject>> {
        let dir = pack.dir.join(&inst.dir);
        Ok(Some(match &inst.kind {
            Kind::Wasm { .. } | Kind::Image { .. } => return Ok(None),
            Kind::Console => EnvObject::Console(Console::default()),
            Kind::MemoryConsole => EnvObject::MemoryConsole(MemoryConsole::default()),
            Kind::Tcp => EnvObject::Tcp(Tcp::bind(&config(inst)?)?),
            Kind::MemoryListener => EnvObject::MemoryListener(MemoryListener::default()),
            Kind::Files => EnvObject::Files(Files::open(dir.join(config(inst)?))?),
            Kind::Packed { .. } => {
                let tree = pack.trees.get(&inst.name).context("its files are missing")?.clone();
                EnvObject::Packed(Packed::new(tree, pack.blobs.clone()))
            }
        }))
    }

    /// Starts listening for outside events. Called once the image is set up.
    pub fn start(&mut self, me: usize, events: &Sender<Incoming>) {
        if let EnvObject::Tcp(t) = self {
            t.start(me, events);
        }
    }

    /// Serves request `req`. `out` is for printing.
    pub fn serve(&mut self, req: u32, verb: Verb, path: &str, body: Option<&Payload>, out: &mut dyn Write) -> Result<Vec<Reply>> {
        // Their config is taken from the image's write of their root before
        // they are made; changing it while they run isn't supported yet.
        let configured = matches!(self, EnvObject::Tcp(_) | EnvObject::Files(_) | EnvObject::MemoryListener(_));
        if configured && verb == Verb::Write && path == "/" {
            return Ok(vec![Reply::fail(req, Status::NotSupported)]);
        }
        Ok(match self {
            EnvObject::Console(c) => c.serve(req, verb, path, body, out)?,
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

/// An environment object's config, which is text.
fn config(inst: &Instance) -> Result<String> {
    inst.config.as_ref().context("no config")?.text()
}
