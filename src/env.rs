//! Environmental objects: the only things in an image that touch the outside
//! world. Each is configured by the image alone and exposes its capability
//! through messages.
//!
//! Objects that wait on the outside world (stdin, sockets) do the waiting on
//! their own threads and report [`Event`]s over a channel. All message
//! delivery still happens on the runtime's single thread.

use crate::check::{Kind, LINE, Program};
use crate::messages::FieldType;
use crate::render;
use crate::wire::{self, Message};
use anyhow::{Context, Result, anyhow, bail};
use bson::{Document, doc};
use std::collections::HashMap;
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
    Accepted { conn: u32, stream: TcpStream },
    Data { conn: u32, data: Vec<u8> },
    Closed { conn: u32 },
}

/// A message an environmental object emits on its out port `.0`.
pub type Emit = (u32, Message);

/// Idle connections are dropped after this long without data.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// A peer that won't accept our writes for this long is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub enum EnvObject {
    Stdin,
    Stdout,
    Socket(Socket),
    Files(Files),
}

pub struct Socket {
    address: String,
    streams: HashMap<u32, TcpStream>,
}

pub struct Files {
    /// Canonical, so symlinks can be checked against it.
    root: PathBuf,
}

fn msg(type_name: &str, fields: Document) -> Message {
    Message { type_name: type_name.into(), fields }
}

fn u32_field(v: u32) -> bson::Bson {
    wire::int_to_bson(FieldType::U32, v as i128)
}

impl EnvObject {
    /// `None` for instances implemented in Wasm.
    pub fn new(kind: &Kind) -> Result<Option<EnvObject>> {
        Ok(Some(match kind {
            Kind::Wasm { .. } => return Ok(None),
            Kind::Stdin => EnvObject::Stdin,
            Kind::Stdout => EnvObject::Stdout,
            Kind::Socket { address } => EnvObject::Socket(Socket { address: address.clone(), streams: HashMap::new() }),
            Kind::Files { root } => {
                let root = root.canonicalize().with_context(|| format!("files root {}", root.display()))?;
                if !root.is_dir() {
                    bail!("files root {} is not a directory", root.display());
                }
                EnvObject::Files(Files { root })
            }
        }))
    }

    /// Starts listening for outside events. Called once startup messages
    /// have been delivered.
    pub fn start(&mut self, me: usize, events: &Sender<Event>) -> Result<Vec<Emit>> {
        let EnvObject::Socket(s) = self else { return Ok(vec![]) };
        let listener = TcpListener::bind(&s.address).with_context(|| format!("listening on {}", s.address))?;
        let bound = listener.local_addr()?.to_string();
        let events = events.clone();
        std::thread::spawn(move || accept_loop(listener, me, events));
        Ok(vec![(1, msg("Listening", doc! { "address": bound }))])
    }

    /// Handles a message wired to this object.
    pub fn deliver(&mut self, m: &Message, program: &Program, out: &mut dyn Write) -> Result<Vec<Emit>> {
        let field = |name: &str| m.fields.get(name).ok_or_else(|| anyhow!("`{}` has no `{name}`", m.type_name));
        let conn_of = || field("conn").and_then(int_u32);
        match (self, m.type_name.as_str()) {
            (EnvObject::Stdout, _) => {
                let line = render::render(m, &program.messages);
                writeln!(out, "{line}").and_then(|_| out.flush()).context("writing stdout")?;
                Ok(vec![])
            }
            (EnvObject::Socket(s), "SocketWrite") => {
                let conn = conn_of()?;
                let data = match field("data")? {
                    bson::Bson::Binary(b) => &b.bytes,
                    _ => bail!("SocketWrite.data is not binary"),
                };
                if let Some(stream) = s.streams.get_mut(&conn)
                    && let Err(e) = stream.write_all(data)
                {
                    eprintln!("socket: write to connection {conn} failed: {e}");
                    s.close(conn);
                }
                Ok(vec![])
            }
            (EnvObject::Socket(s), "SocketClose") => {
                s.close(conn_of()?);
                Ok(vec![])
            }
            (EnvObject::Files(f), "ReadFile") => {
                let tag = u32_field(int_u32(field("tag")?)?);
                let path = field("path")?.as_str().unwrap_or_default().to_string();
                Ok(vec![(
                    0,
                    match f.read(&path) {
                        Ok(data) => msg("FileContents", doc! { "tag": tag, "path": path, "data": wire::bytes(data) }),
                        Err(reason) => msg("FileError", doc! { "tag": tag, "path": path, "reason": reason }),
                    },
                )])
            }
            (_, other) => bail!("environment object cannot handle `{other}` at runtime"),
        }
    }

    /// Turns an outside event into messages, updating state on the way.
    pub fn on_event(&mut self, what: What) -> Vec<Emit> {
        match (self, what) {
            (EnvObject::Stdin, What::Line(text)) => vec![(0, msg(LINE, doc! { "text": text }))],
            (EnvObject::Socket(s), What::Accepted { conn, stream }) => {
                s.streams.insert(conn, stream);
                vec![]
            }
            (EnvObject::Socket(_), What::Data { conn, data }) => {
                vec![(0, msg("SocketData", doc! { "conn": u32_field(conn), "data": wire::bytes(data) }))]
            }
            (EnvObject::Socket(s), What::Closed { conn }) => {
                s.streams.remove(&conn);
                vec![(0, msg("SocketClosed", doc! { "conn": u32_field(conn) }))]
            }
            _ => unreachable!("event for the wrong kind of object"),
        }
    }
}

fn int_u32(v: &bson::Bson) -> Result<u32> {
    match v {
        bson::Bson::Int32(i) => Ok(*i as u32),
        bson::Bson::Int64(i) => Ok(*i as u32),
        other => bail!("expected an integer, got {other}"),
    }
}

/// Reads lines and hands them to the stdin instances in turn, until EOF.
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
                break;
            }
        }
    });
}

fn accept_loop(listener: TcpListener, to: usize, events: Sender<Event>) {
    for (conn, stream) in (1u32..).zip(listener.incoming()) {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("socket: accept failed: {e}");
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

impl Socket {
    fn close(&mut self, conn: u32) {
        if let Some(stream) = self.streams.remove(&conn) {
            // Only our half: the reader sees the peer's EOF and reports Closed.
            let _ = stream.shutdown(Shutdown::Write);
        }
    }
}

impl Files {
    /// Reads `path` relative to the root, or explains why not. Never leaves
    /// the root: `..` is refused and symlinks are resolved and re-checked.
    fn read(&self, path: &str) -> std::result::Result<Vec<u8>, &'static str> {
        let full = resolve(&self.root, path)?;
        let real = full.canonicalize().map_err(|_| "not_found")?;
        if !real.starts_with(&self.root) {
            return Err("forbidden");
        }
        if real.is_dir() {
            return Err("not_found");
        }
        std::fs::read(&real).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "not_found" } else { "io" })
    }
}

fn resolve(root: &Path, path: &str) -> std::result::Result<PathBuf, &'static str> {
    if path.contains('\0') || path.contains('\\') {
        return Err("forbidden");
    }
    let mut out = root.to_path_buf();
    for c in Path::new(path.trim_start_matches('/')).components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return Err("forbidden"),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_stay_inside_the_root() {
        let base = std::env::temp_dir().join(format!("real_talk_files_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("root/sub")).unwrap();
        std::fs::write(base.join("root/a.txt"), "A").unwrap();
        std::fs::write(base.join("root/sub/b.txt"), "B").unwrap();
        std::fs::write(base.join("secret.txt"), "S").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("secret.txt"), base.join("root/link.txt")).unwrap();

        let f = Files { root: base.join("root").canonicalize().unwrap() };
        assert_eq!(f.read("/a.txt").unwrap(), b"A");
        assert_eq!(f.read("sub/./b.txt").unwrap(), b"B");
        assert_eq!(f.read("/sub"), Err("not_found"));
        assert_eq!(f.read("/missing"), Err("not_found"));
        assert_eq!(f.read("/../secret.txt"), Err("forbidden"));
        assert_eq!(f.read("sub/../../secret.txt"), Err("forbidden"));
        assert_eq!(f.read("sub\\..\\a.txt"), Err("forbidden"));
        #[cfg(unix)]
        assert_eq!(f.read("/link.txt"), Err("forbidden"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
