//! `Listener`, run as a TCP listener and the connections it accepts.

use super::{Event, Reply, What, msg};
use crate::ns::{Status, Verb};
use crate::wire::{self, Message};
use anyhow::{Context, Result};
use bson::{Bson, doc};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Idle connections are dropped after this long without data.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// A peer that won't accept our writes for this long is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

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

fn data_msg(data: Vec<u8>) -> Message {
    msg("Data", doc! { "mime": "", "data": wire::bytes(data) })
}

impl Tcp {
    /// Binds `address` now, so the bound address is known before anything starts.
    pub(super) fn bind(address: &str) -> Result<Tcp> {
        let listener = TcpListener::bind(address).with_context(|| format!("listening on {address}"))?;
        let address = listener.local_addr()?.to_string();
        Ok(Tcp { listener: Some(listener), address, accept_watchers: vec![], conns: HashMap::new() })
    }

    /// Starts accepting connections, on a thread of its own.
    pub(super) fn start(&mut self, me: usize, events: &Sender<Event>) {
        if let Some(listener) = self.listener.take() {
            let events = events.clone();
            std::thread::spawn(move || accept_loop(listener, me, events));
        }
    }

    pub(super) fn serve(&mut self, req: u32, verb: Verb, path: &str, body: Option<&Message>) -> Vec<Reply> {
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

    pub(super) fn cancel(&mut self, req: u32) {
        self.accept_watchers.retain(|&w| w != req);
        for conn in self.conns.values_mut() {
            if conn.watcher == Some(req) {
                conn.watcher = None;
            }
        }
    }

    pub(super) fn on_event(&mut self, what: What) -> Vec<Reply> {
        match what {
            What::Accepted { conn, stream } => {
                let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
                self.conns.insert(conn, Conn { stream, watcher: None, pending: vec![], eof: false, shut: false });
                let accepted = msg("Accepted", doc! { "conn": conn.to_string(), "peer": peer });
                self.accept_watchers.iter().map(|&w| Reply::more(w, accepted.clone())).collect()
            }
            What::Data { conn, data } => {
                let Some(c) = self.conns.get_mut(&conn) else { return vec![] };
                match c.watcher {
                    Some(w) => vec![Reply::more(w, data_msg(data))],
                    None => {
                        c.pending.push(data);
                        vec![]
                    }
                }
            }
            What::Closed { conn } => {
                let Some(c) = self.conns.get_mut(&conn) else { return vec![] };
                c.eof = true;
                let replies = c.watcher.take().map(Reply::end).into_iter().collect();
                self.forget_if_finished(conn);
                replies
            }
            _ => unreachable!("only sockets report to a listener"),
        }
    }
}

/// A listener run in memory, for an imported image: connections come from
/// whoever creates `/dial`, with the request's bytes as what the server
/// receives, and everything the server writes streams back to the dialer
/// until the server removes the connection. Nothing touches the host.
#[derive(Default)]
pub struct MemoryListener {
    /// Requests watching `/accept`.
    accept_watchers: Vec<u32>,
    /// Dialed before anyone watched `/accept`, so not yet announced.
    unannounced: Vec<u32>,
    conns: HashMap<u32, MemoryConn>,
    next: u32,
}

struct MemoryConn {
    /// The `/dial` request, which the server's writes answer.
    dialer: u32,
    /// The server's request watching `/<conn>`, if any.
    watcher: Option<u32>,
    /// Sent by the dialer before the server watched.
    pending: Vec<Vec<u8>>,
}

fn accepted(conn: u32) -> Message {
    msg("Accepted", doc! { "conn": conn.to_string(), "peer": "memory" })
}

/// The bytes of a `Data` body.
fn data_of(body: Option<&Message>) -> Option<&[u8]> {
    match body.map(|m| (m.type_name.as_str(), m.fields.get("data"))) {
        Some(("Data", Some(Bson::Binary(b)))) => Some(&b.bytes),
        _ => None,
    }
}

impl MemoryListener {
    pub(super) fn serve(&mut self, req: u32, verb: Verb, path: &str, body: Option<&Message>) -> Vec<Reply> {
        match (verb, path) {
            (Verb::Read, "/address") => return vec![Reply::ok(req, msg("Text", doc! { "text": "memory" }))],
            (Verb::Watch, "/accept") => {
                self.accept_watchers.push(req);
                return self.unannounced.drain(..).map(|conn| Reply::more(req, accepted(conn))).collect();
            }
            (Verb::Create, "/dial") => {
                let Some(data) = data_of(body) else { return vec![Reply::fail(req, Status::Invalid)] };
                self.next += 1;
                let conn = self.next;
                self.conns.insert(conn, MemoryConn { dialer: req, watcher: None, pending: vec![data.to_vec()] });
                if self.accept_watchers.is_empty() {
                    self.unannounced.push(conn);
                }
                return self.accept_watchers.iter().map(|&w| Reply::more(w, accepted(conn))).collect();
            }
            (_, "/address" | "/accept" | "/dial") => return vec![Reply::fail(req, Status::NotSupported)],
            _ => {}
        }
        let Some(conn) = path.strip_prefix('/').and_then(|n| n.parse::<u32>().ok()) else {
            return vec![Reply::fail(req, Status::NotFound)];
        };
        let Some(c) = self.conns.get_mut(&conn) else { return vec![Reply::fail(req, Status::NotFound)] };
        match verb {
            Verb::Watch if c.watcher.is_some() => vec![Reply::fail(req, Status::Exists)],
            Verb::Watch => {
                c.watcher = Some(req);
                c.pending.drain(..).map(|d| Reply::more(req, data_msg(d))).collect()
            }
            Verb::Write => match data_of(body) {
                Some(data) => vec![Reply::more(c.dialer, data_msg(data.to_vec())), Reply::end(req)],
                None => vec![Reply::fail(req, Status::Invalid)],
            },
            // The server hangs up: the dialer has everything, and the server
            // hears the dialer hang up too.
            Verb::Remove => {
                let c = self.conns.remove(&conn).expect("just found");
                let mut replies = vec![Reply::end(c.dialer)];
                replies.extend(c.watcher.map(Reply::end));
                replies.push(Reply::end(req));
                replies
            }
            _ => vec![Reply::fail(req, Status::NotSupported)],
        }
    }

    /// A dialer that gives up hangs up its side; the server hears it close.
    pub(super) fn cancel(&mut self, req: u32) -> Vec<Reply> {
        self.accept_watchers.retain(|&w| w != req);
        let mut replies = vec![];
        if let Some(&conn) = self.conns.iter().find(|(_, c)| c.dialer == req).map(|(n, _)| n) {
            let c = self.conns.remove(&conn).expect("just found");
            self.unannounced.retain(|&n| n != conn);
            replies.extend(c.watcher.map(Reply::end));
        }
        for c in self.conns.values_mut() {
            if c.watcher == Some(req) {
                c.watcher = None;
            }
        }
        replies
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn data(bytes: &str) -> Message {
        data_msg(bytes.as_bytes().to_vec())
    }

    #[test]
    fn a_dial_is_a_connection_the_server_answers() {
        let mut l = MemoryListener::default();
        // Dialed before anyone watches: announced to the first watcher.
        assert!(l.serve(1, Verb::Create, "/dial", Some(&data("GET /"))).is_empty());
        assert_eq!(l.serve(2, Verb::Watch, "/accept", None), [Reply::more(2, accepted(1))]);
        assert_eq!(l.serve(3, Verb::Watch, "/1", None), [Reply::more(3, data("GET /"))]);
        assert_eq!(l.serve(4, Verb::Write, "/1", Some(&data("200"))), [Reply::more(1, data("200")), Reply::end(4)]);
        assert_eq!(l.serve(5, Verb::Remove, "/1", None), [Reply::end(1), Reply::end(3), Reply::end(5)]);
        assert_eq!(l.serve(6, Verb::Watch, "/1", None)[0].status, Status::NotFound);

        // Dialed while watched: announced at once.
        assert_eq!(l.serve(7, Verb::Create, "/dial", Some(&data("x"))), [Reply::more(2, accepted(2))]);
        assert_eq!(l.serve(8, Verb::Watch, "/2", None), [Reply::more(8, data("x"))]);
        assert_eq!(l.cancel(7), [Reply::end(8)], "the dialer hung up");
        assert_eq!(l.serve(9, Verb::Create, "/dial", None)[0].status, Status::Invalid);
        assert_eq!(l.serve(10, Verb::Read, "/address", None)[0].body, Some(msg("Text", doc! { "text": "memory" })));
    }
}
