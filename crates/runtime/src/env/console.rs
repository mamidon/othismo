//! `Console`, run on the host's stdin and stdout.

use super::{Event, Reply, What};
use protocol::ns::{Status, Verb};
use protocol::payload::Payload;
use crate::render;
use anyhow::{Context, Result};
use std::io::{BufRead, Write};
use std::sync::mpsc::Sender;

#[derive(Default)]
pub struct Console {
    /// Requests watching `/`.
    watchers: Vec<u32>,
    eof: bool,
}

impl Console {
    /// Whether anything is watching stdin through this console.
    pub fn is_watched(&self) -> bool {
        !self.watchers.is_empty()
    }

    pub(super) fn serve(&mut self, req: u32, verb: Verb, path: &str, body: Option<&Payload>, out: &mut dyn Write) -> Result<Vec<Reply>> {
        if path != "/" {
            return Ok(vec![Reply::fail(req, Status::NotFound)]);
        }
        Ok(match (verb, body) {
            (Verb::Watch, _) if self.eof => vec![Reply::end(req)],
            (Verb::Watch, _) => {
                self.watchers.push(req);
                vec![]
            }
            (Verb::Write, Some(p)) => {
                writeln!(out, "{}", render::line(p)).and_then(|_| out.flush()).context("writing stdout")?;
                vec![Reply::end(req)]
            }
            (Verb::Write, None) => vec![Reply::fail(req, Status::Invalid)],
            _ => vec![Reply::fail(req, Status::NotSupported)],
        })
    }

    pub(super) fn cancel(&mut self, req: u32) {
        self.watchers.retain(|&w| w != req);
    }

    pub(super) fn on_event(&mut self, what: What) -> Vec<Reply> {
        match what {
            What::Line(text) => self.watchers.iter().map(|&w| Reply::more(w, Payload::text(&text))).collect(),
            What::Eof => {
                self.eof = true;
                self.watchers.drain(..).map(Reply::end).collect()
            }
            _ => unreachable!("only stdin reports to a console"),
        }
    }
}

/// A console run in memory, for an imported image: lines come from whoever
/// writes to `/typed`, and whatever is written goes to whoever watches
/// `/printed`. Nothing touches the host.
#[derive(Default)]
pub struct MemoryConsole {
    /// Requests watching `/`.
    watchers: Vec<u32>,
    /// Requests watching `/printed`.
    printed: Vec<u32>,
}

impl MemoryConsole {
    pub(super) fn serve(&mut self, req: u32, verb: Verb, path: &str, body: Option<&Payload>) -> Vec<Reply> {
        match (path, verb, body) {
            ("/", Verb::Watch, _) => {
                self.watchers.push(req);
                vec![]
            }
            ("/", Verb::Write, Some(p)) => {
                let mut replies: Vec<Reply> = self.printed.iter().map(|&w| Reply::more(w, p.clone())).collect();
                replies.push(Reply::end(req));
                replies
            }
            ("/typed", Verb::Write, Some(p)) => {
                let mut replies: Vec<Reply> = self.watchers.iter().map(|&w| Reply::more(w, p.clone())).collect();
                replies.push(Reply::end(req));
                replies
            }
            ("/printed", Verb::Watch, _) => {
                self.printed.push(req);
                vec![]
            }
            ("/" | "/typed", Verb::Write, None) => vec![Reply::fail(req, Status::Invalid)],
            ("/" | "/typed" | "/printed", ..) => vec![Reply::fail(req, Status::NotSupported)],
            _ => vec![Reply::fail(req, Status::NotFound)],
        }
    }

    pub(super) fn cancel(&mut self, req: u32) {
        self.watchers.retain(|&w| w != req);
        self.printed.retain(|&w| w != req);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::EnvObject;

    fn status(replies: Vec<Reply>) -> Vec<Status> {
        replies.into_iter().map(|r| r.status).collect()
    }

    #[test]
    fn console_streams_lines_to_watchers_until_eof() {
        let mut c = EnvObject::Console(Console::default());
        let mut out = Vec::new();
        assert!(c.serve(1, Verb::Watch, "/", None, &mut out).unwrap().is_empty());
        assert!(c.serve(2, Verb::Watch, "/", None, &mut out).unwrap().is_empty());
        assert_eq!(c.cancel(2), [Reply::end(2)]);
        let r = c.on_event(What::Line("hi".into()));
        assert_eq!(r, [Reply::more(1, Payload::text("hi"))]);
        assert_eq!(c.on_event(What::Eof), [Reply::end(1)]);
        assert_eq!(c.serve(3, Verb::Watch, "/", None, &mut out).unwrap(), [Reply::end(3)]);

        let w = c.serve(4, Verb::Write, "/", Some(&Payload::text("out")), &mut out).unwrap();
        assert_eq!(w, [Reply::end(4)]);
        assert_eq!(out, b"out\n");
        assert_eq!(status(c.serve(5, Verb::Write, "/x", None, &mut out).unwrap()), [Status::NotFound]);
    }

    #[test]
    fn a_memory_console_is_typed_into_and_printed_from() {
        let mut c = MemoryConsole::default();
        assert!(c.serve(1, Verb::Watch, "/", None).is_empty());
        assert!(c.serve(2, Verb::Watch, "/printed", None).is_empty());
        let line = Payload::text("hi");
        assert_eq!(c.serve(3, Verb::Write, "/typed", Some(&line)), [Reply::more(1, line.clone()), Reply::end(3)]);
        let out = Payload::text("out");
        assert_eq!(c.serve(4, Verb::Write, "/", Some(&out)), [Reply::more(2, out), Reply::end(4)]);
        c.cancel(2);
        assert_eq!(c.serve(5, Verb::Write, "/", Some(&line)), [Reply::end(5)]);
        assert_eq!(status(c.serve(6, Verb::Read, "/printed", None)), [Status::NotSupported]);
        assert_eq!(status(c.serve(7, Verb::Read, "/x", None)), [Status::NotFound]);
    }
}
