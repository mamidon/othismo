//! Environmental objects: the only things in an image that touch the outside
//! world. Each is a server the runtime implements, configured by the image
//! alone, serving requests made of its tree.
//!
//! Objects that wait on the outside world (stdin, sockets) do the waiting on
//! their own threads and report [`Event`]s over a channel. All requests and
//! replies still happen on the runtime's single thread.

use crate::check::Kind;
use crate::messages::MessageType;
use crate::ns::{Status, Verb};
use crate::render;
use crate::wire::{self, Message};
use anyhow::{Context, Result, bail};
use bson::{Bson, Document, doc};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

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

/// Idle connections are dropped after this long without data.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// A peer that won't accept our writes for this long is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub enum EnvObject {
    Console(Console),
    Tcp(Tcp),
    Files(Files),
}

#[derive(Default)]
pub struct Console {
    /// Requests watching `/`.
    watchers: Vec<u32>,
    eof: bool,
}

pub struct Tcp {
    /// Taken by `start`, which hands it to the accept thread.
    listener: Option<TcpListener>,
    address: String,
    /// Requests watching `/accept`.
    accept_watchers: Vec<u32>,
    conns: HashMap<u32, Conn>,
}

struct Conn {
    stream: TcpStream,
    /// The request watching `/<conn>`, if any.
    watcher: Option<u32>,
    /// Received before anyone watched.
    pending: Vec<Vec<u8>>,
    /// The peer is done sending.
    eof: bool,
    /// We are done sending.
    shut: bool,
}

pub struct Files {
    /// Canonical, so symlinks can be checked against it.
    root: PathBuf,
}

fn msg(type_name: &str, fields: Document) -> Message {
    Message { type_name: type_name.into(), fields }
}

impl EnvObject {
    /// `None` for instances implemented in Wasm. A Tcp listener is bound
    /// here, so its address is known before anything starts.
    pub fn new(kind: &Kind) -> Result<Option<EnvObject>> {
        Ok(Some(match kind {
            Kind::Wasm { .. } => return Ok(None),
            Kind::Console => EnvObject::Console(Console::default()),
            Kind::Tcp { address } => {
                let listener = TcpListener::bind(address).with_context(|| format!("listening on {address}"))?;
                let address = listener.local_addr()?.to_string();
                EnvObject::Tcp(Tcp { listener: Some(listener), address, accept_watchers: vec![], conns: HashMap::new() })
            }
            Kind::Files { root } => {
                let root = root.canonicalize().with_context(|| format!("files root {}", root.display()))?;
                if !root.is_dir() {
                    bail!("files root {} is not a directory", root.display());
                }
                EnvObject::Files(Files { root })
            }
        }))
    }

    /// Starts listening for outside events. Called once the image is set up.
    pub fn start(&mut self, me: usize, events: &Sender<Event>) {
        if let EnvObject::Tcp(t) = self
            && let Some(listener) = t.listener.take()
        {
            let events = events.clone();
            std::thread::spawn(move || accept_loop(listener, me, events));
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
        Ok(match self {
            EnvObject::Console(c) => c.serve(req, verb, path, body, out, messages)?,
            EnvObject::Tcp(t) => t.serve(req, verb, path, body),
            EnvObject::Files(f) => f.serve(req, verb, path),
        })
    }

    /// The caller gave up on `req`.
    pub fn cancel(&mut self, req: u32) -> Vec<Reply> {
        match self {
            EnvObject::Console(c) => c.watchers.retain(|&w| w != req),
            EnvObject::Tcp(t) => {
                t.accept_watchers.retain(|&w| w != req);
                for conn in t.conns.values_mut() {
                    if conn.watcher == Some(req) {
                        conn.watcher = None;
                    }
                }
            }
            EnvObject::Files(_) => {}
        }
        vec![Reply::end(req)]
    }

    /// Turns an outside event into replies, updating state on the way.
    pub fn on_event(&mut self, what: What) -> Vec<Reply> {
        match (self, what) {
            (EnvObject::Console(c), What::Line(text)) => {
                c.watchers.iter().map(|&w| Reply::more(w, msg("Line", doc! { "text": text.clone() }))).collect()
            }
            (EnvObject::Console(c), What::Eof) => {
                c.eof = true;
                c.watchers.drain(..).map(Reply::end).collect()
            }
            (EnvObject::Tcp(t), What::Accepted { conn, stream }) => {
                let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
                t.conns.insert(conn, Conn { stream, watcher: None, pending: vec![], eof: false, shut: false });
                let accepted = msg("Accepted", doc! { "conn": conn.to_string(), "peer": peer });
                t.accept_watchers.iter().map(|&w| Reply::more(w, accepted.clone())).collect()
            }
            (EnvObject::Tcp(t), What::Data { conn, data }) => {
                let Some(c) = t.conns.get_mut(&conn) else { return vec![] };
                match c.watcher {
                    Some(w) => vec![Reply::more(w, data_msg(data))],
                    None => {
                        c.pending.push(data);
                        vec![]
                    }
                }
            }
            (EnvObject::Tcp(t), What::Closed { conn }) => {
                let Some(c) = t.conns.get_mut(&conn) else { return vec![] };
                c.eof = true;
                let replies = c.watcher.take().map(Reply::end).into_iter().collect();
                t.forget_if_finished(conn);
                replies
            }
            _ => unreachable!("event for the wrong kind of object"),
        }
    }
}

fn data_msg(data: Vec<u8>) -> Message {
    msg("Data", doc! { "mime": "", "data": wire::bytes(data) })
}

impl Console {
    /// Whether anything is watching stdin through this console.
    pub fn is_watched(&self) -> bool {
        !self.watchers.is_empty()
    }

    fn serve(
        &mut self,
        req: u32,
        verb: Verb,
        path: &str,
        body: Option<&Message>,
        out: &mut dyn Write,
        messages: &BTreeMap<String, MessageType>,
    ) -> Result<Vec<Reply>> {
        if path != "/" {
            return Ok(vec![Reply::fail(req, Status::NotFound)]);
        }
        Ok(match (verb, body) {
            (Verb::Watch, _) if self.eof => vec![Reply::end(req)],
            (Verb::Watch, _) => {
                self.watchers.push(req);
                vec![]
            }
            (Verb::Write, Some(m)) => {
                let line = render::render(m, messages);
                writeln!(out, "{line}").and_then(|_| out.flush()).context("writing stdout")?;
                vec![Reply::end(req)]
            }
            (Verb::Write, None) => vec![Reply::fail(req, Status::Invalid)],
            _ => vec![Reply::fail(req, Status::NotSupported)],
        })
    }
}

impl Tcp {
    fn serve(&mut self, req: u32, verb: Verb, path: &str, body: Option<&Message>) -> Vec<Reply> {
        match (verb, path) {
            (Verb::Read, "/address") => return vec![Reply::ok(req, msg("Text", doc! { "text": self.address.clone() }))],
            (Verb::Watch, "/accept") => {
                self.accept_watchers.push(req);
                return vec![];
            }
            (_, "/address" | "/accept") => return vec![Reply::fail(req, Status::NotSupported)],
            _ => {}
        }
        let Some(conn) = path.strip_prefix('/').and_then(|n| n.parse::<u32>().ok()) else {
            return vec![Reply::fail(req, Status::NotFound)];
        };
        let Some(c) = self.conns.get_mut(&conn) else { return vec![Reply::fail(req, Status::NotFound)] };
        match verb {
            Verb::Watch if c.watcher.is_some() => vec![Reply::fail(req, Status::Exists)],
            Verb::Watch => {
                let mut replies: Vec<Reply> = c.pending.drain(..).map(|d| Reply::more(req, data_msg(d))).collect();
                if c.eof {
                    replies.push(Reply::end(req));
                    self.forget_if_finished(conn);
                } else {
                    c.watcher = Some(req);
                }
                replies
            }
            Verb::Write => {
                let data = match body.map(|m| (m.type_name.as_str(), m.fields.get("data"))) {
                    Some(("Data", Some(Bson::Binary(b)))) => &b.bytes,
                    _ => return vec![Reply::fail(req, Status::Invalid)],
                };
                if c.shut {
                    return vec![Reply::fail(req, Status::Invalid)];
                }
                match c.stream.write_all(data) {
                    Ok(()) => vec![Reply::end(req)],
                    Err(e) => {
                        eprintln!("tcp: write to connection {conn} failed: {e}");
                        vec![Reply::fail(req, Status::Io)]
                    }
                }
            }
            Verb::Remove => {
                // Only our half: the reader sees the peer's EOF and reports Closed.
                let _ = c.stream.shutdown(Shutdown::Write);
                c.shut = true;
                self.forget_if_finished(conn);
                vec![Reply::end(req)]
            }
            _ => vec![Reply::fail(req, Status::NotSupported)],
        }
    }

    /// Drops a connection once both sides are done with it.
    fn forget_if_finished(&mut self, conn: u32) {
        if self.conns.get(&conn).is_some_and(|c| c.eof && c.shut && c.watcher.is_none()) {
            self.conns.remove(&conn);
        }
    }
}

/// Reads lines and hands them to the Console instances in turn, until EOF,
/// which every Console is told about.
pub fn spawn_stdin(mut input: Box<dyn BufRead + Send>, instances: Vec<usize>, events: Sender<Event>) {
    std::thread::spawn(move || {
        for &to in instances.iter().cycle() {
            let mut line = String::new();
            match input.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    eprintln!("stdin: {e}");
                    break;
                }
            }
            let text = line.strip_suffix('\n').unwrap_or(&line);
            let text = text.strip_suffix('\r').unwrap_or(text);
            if events.send(Event { to, what: What::Line(text.into()) }).is_err() {
                return;
            }
        }
        for &to in &instances {
            let _ = events.send(Event { to, what: What::Eof });
        }
    });
}

fn accept_loop(listener: TcpListener, to: usize, events: Sender<Event>) {
    for (conn, stream) in (1u32..).zip(listener.incoming()) {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("tcp: accept failed: {e}");
                continue;
            }
        };
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let Ok(reader) = stream.try_clone() else { continue };
        // Accepted is sent before the reader starts, so it arrives first.
        if events.send(Event { to, what: What::Accepted { conn, stream } }).is_err() {
            return;
        }
        let events = events.clone();
        std::thread::spawn(move || read_loop(reader, to, conn, events));
    }
}

fn read_loop(mut stream: TcpStream, to: usize, conn: u32, events: Sender<Event>) {
    let mut buf = vec![0; 64 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if events.send(Event { to, what: What::Data { conn, data: buf[..n].to_vec() } }).is_err() {
                    return;
                }
            }
        }
    }
    let _ = events.send(Event { to, what: What::Closed { conn } });
}

impl Files {
    fn serve(&self, req: u32, verb: Verb, path: &str) -> Vec<Reply> {
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
fn mime(path: &Path) -> &'static str {
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
        let base = std::env::temp_dir().join(format!("real_talk_files_{}", std::process::id()));
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

    #[test]
    fn console_streams_lines_to_watchers_until_eof() {
        let mut c = EnvObject::Console(Console::default());
        let mut out = Vec::new();
        let messages = BTreeMap::new();
        assert!(c.serve(1, Verb::Watch, "/", None, &mut out, &messages).unwrap().is_empty());
        assert!(c.serve(2, Verb::Watch, "/", None, &mut out, &messages).unwrap().is_empty());
        assert_eq!(c.cancel(2), [Reply::end(2)]);
        let r = c.on_event(What::Line("hi".into()));
        assert_eq!(r, [Reply::more(1, msg("Line", doc! { "text": "hi" }))]);
        assert_eq!(c.on_event(What::Eof), [Reply::end(1)]);
        assert_eq!(c.serve(3, Verb::Watch, "/", None, &mut out, &messages).unwrap(), [Reply::end(3)]);

        let w = c.serve(4, Verb::Write, "/", Some(&msg("Text", doc! { "text": "out" })), &mut out, &messages).unwrap();
        assert_eq!(w, [Reply::end(4)]);
        assert_eq!(out, b"out\n");
        assert_eq!(status(c.serve(5, Verb::Write, "/x", None, &mut out, &messages).unwrap()), [Status::NotFound]);
    }
}
