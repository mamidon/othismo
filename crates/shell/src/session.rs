//! A running image and where the operator is looking from.
//!
//! The image runs on a thread of its own, in this process: the shell holds
//! the only [`Operator`] for it, so nothing but the person at this terminal
//! can make requests through it. What the image prints, and the answers to
//! the shell's requests, come back as [`Event`]s.

use crate::command::{self, Builtin, Line};
use crate::world::{Hop, Viewpoint, World};
use anyhow::{Result, anyhow, bail};
use format::Formats;
use image::pack::Pack;
use protocol::verb::Verb;
use protocol::payload::Payload;
use runtime::operator::{self, Command, Notice, Operator};
use runtime::{Answered, Io, Options, Runtime};
use std::collections::BTreeMap;
use std::io::{BufReader, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

/// What happened in the running image.
#[derive(Debug)]
pub enum Event {
    /// An answer to request `id`.
    Answer { id: u64, answer: Answered },
    /// The world has been brought up to date.
    Refreshed,
    /// A line the image printed on its console.
    Printed(String),
    /// The run ended, with why if it failed.
    Stopped(Option<String>),
}

/// A request that hasn't had its last answer.
#[derive(Debug, Clone)]
pub struct Open {
    pub from: Viewpoint,
    pub what: String,
}

/// What a typed line did.
#[derive(Debug)]
pub enum Outcome {
    Nothing,
    /// A request was sent, as `id`.
    Sent(u64),
    /// Lines for the operator to read.
    Said(Vec<String>),
    Quit,
}

pub struct Session {
    pub world: World,
    pub formats: Formats,
    /// The root looked from first, whose routes are shown; `None` at the image.
    pub entered: Option<Viewpoint>,
    /// Where requests are made from; `None` at the image.
    pub here: Option<Viewpoint>,
    pub open: BTreeMap<u64, Open>,
    pub stopped: Option<Option<String>>,
    op: Operator,
    events: Receiver<Raw>,
    typed: Option<Sender<String>>,
    run: Option<JoinHandle<()>>,
    next: u64,
}

/// What arrives from the image's thread, before the session makes sense of it.
enum Raw {
    Notice(Notice),
    Printed(String),
    Stopped(Option<String>),
}

impl Session {
    /// Runs the image `load` makes, on a thread of its own, and waits for
    /// it to start. `load` runs on that thread: a pack stays on one.
    pub fn start(load: impl FnOnce() -> Result<Pack> + Send + 'static, options: Options, formats: Formats) -> Result<Session> {
        let (op, inbox) = operator::channel();
        let (tx, events) = mpsc::channel();
        let (typed, lines) = mpsc::channel::<String>();
        let run = std::thread::Builder::new().name("image".into()).spawn({
            let tx = tx.clone();
            move || {
                let result = (|| {
                    let pack = load()?;
                    let printer = Printer { to: tx.clone(), line: vec![] };
                    let io = Io { input: Box::new(BufReader::new(Typed { lines, pending: vec![] })), output: Box::new(printer) };
                    let notices = tx.clone();
                    Runtime::new(&pack, io, options)?.run_operated(inbox, move |n| {
                        let _ = notices.send(Raw::Notice(n));
                    })
                })();
                let _ = tx.send(Raw::Stopped(result.err().map(|e| format!("{e:#}"))));
            }
        })?;
        op.send(Command::Snapshot);
        // The first snapshot, once it has started; or why it couldn't.
        let world = loop {
            match events.recv_timeout(Duration::from_secs(30)) {
                Ok(Raw::Notice(Notice::Snapshot(s))) => break World::new(s),
                Ok(Raw::Stopped(why)) => bail!("the image stopped before it started: {}", why.unwrap_or_else(|| "it had nothing to do".into())),
                Ok(_) => {}
                Err(_) => bail!("the image didn't start within 30 seconds"),
            }
        };
        Ok(Session {
            world,
            formats,
            entered: None,
            here: None,
            open: BTreeMap::new(),
            stopped: None,
            op,
            events,
            typed: Some(typed),
            run: Some(run),
            next: 0,
        })
    }

    /// Carries out a typed line.
    pub fn run_line(&mut self, line: &str) -> Result<Outcome> {
        match command::parse(line, &self.formats, &self.world.snapshot.messages)? {
            Line::Empty => Ok(Outcome::Nothing),
            Line::Request { verb, path, body } => self.request(verb, &path, body).map(Outcome::Sent),
            Line::Shell(b) => self.builtin(b),
        }
    }

    /// Sends `verb path [body]` from where the operator looks from.
    pub fn request(&mut self, verb: Verb, path: &str, body: Option<Payload>) -> Result<u64> {
        let from = self.here.ok_or_else(|| anyhow!("look from somewhere first: requests are made from a point of view (:roots)"))?;
        self.next += 1;
        let id = self.next;
        let what = format!("{} {path}", verb.keyword());
        if !self.op.send(Command::Request { id, from: from.instance(), verb, path: path.into(), body }) {
            bail!("the image has stopped");
        }
        self.open.insert(id, Open { from, what });
        Ok(id)
    }

    /// Asks instance `inst` itself, as the operator, about `path` in what
    /// it serves: how the shell lists served trees, whoever it looks from.
    /// Only safe verbs; its answers aren't counted among the open requests.
    pub fn look_inside(&mut self, inst: usize, verb: Verb, path: &str) -> Result<u64> {
        if !verb.is_safe() {
            bail!("looking inside changes nothing: {} can't be sent that way", verb.keyword());
        }
        self.next += 1;
        if !self.op.send(Command::Look { id: self.next, inst, verb, path: path.into() }) {
            bail!("the image has stopped");
        }
        Ok(self.next)
    }

    pub fn cancel(&mut self, id: Option<u64>) -> Result<u64> {
        let id = id.or_else(|| self.open.keys().last().copied()).ok_or_else(|| anyhow!("nothing is open"))?;
        if !self.open.contains_key(&id) {
            bail!("request {id} isn't open");
        }
        self.op.send(Command::Cancel { id });
        Ok(id)
    }

    /// Asks for a new snapshot; [`Event::Refreshed`] says when it is in.
    pub fn refresh(&self) {
        self.op.send(Command::Snapshot);
    }

    /// Types a line into the image's consoles, as stdin would.
    pub fn type_line(&self, line: &str) -> Result<()> {
        let typed = self.typed.as_ref().ok_or_else(|| anyhow!("the consoles' input has ended"))?;
        typed.send(line.to_string()).map_err(|_| anyhow!("nothing reads the consoles' input any more"))
    }

    /// The next event, if one comes within `wait`.
    pub fn next_event(&mut self, wait: Duration) -> Option<Event> {
        let raw = match self.events.recv_timeout(wait) {
            Ok(raw) => raw,
            Err(RecvTimeoutError::Timeout) => return None,
            Err(RecvTimeoutError::Disconnected) => return self.stopped.is_none().then(|| self.stop_with(None)),
        };
        Some(match raw {
            Raw::Notice(Notice::Answer { id, answer }) => {
                if answer.done {
                    self.open.remove(&id);
                }
                Event::Answer { id, answer }
            }
            Raw::Notice(Notice::Snapshot(s)) => {
                self.world = World::new(s);
                Event::Refreshed
            }
            Raw::Printed(text) => Event::Printed(text),
            Raw::Stopped(why) => self.stop_with(why),
        })
    }

    fn stop_with(&mut self, why: Option<String>) -> Event {
        self.stopped = Some(why.clone());
        self.open.clear();
        Event::Stopped(why)
    }

    /// Looks from `at`, entering it as a root if it is one, or if the root
    /// entered doesn't hold it.
    pub fn look(&mut self, at: Viewpoint) -> Result<()> {
        if !self.world.has_view(at) {
            bail!("{} has no view to look from: it is one of the runtime's objects", self.world.name(at));
        }
        let entered = match self.entered {
            Some(root) if self.world.route(root, at).is_some() => root,
            _ => self.world.roots().into_iter().find(|&r| self.world.route(r, at).is_some()).unwrap_or(at),
        };
        self.entered = Some(entered);
        self.here = Some(at);
        Ok(())
    }

    /// Looks from the previous step of the route, or the image from a root.
    pub fn up(&mut self) {
        let route = self.route();
        match route.len() {
            0 | 1 => {
                self.here = None;
                self.entered = None;
            }
            n => self.here = Some(route[n - 2].at),
        }
    }

    /// The route from the root entered to where the operator looks from.
    pub fn route(&self) -> Vec<Hop> {
        match (self.entered, self.here) {
            (Some(root), Some(here)) => self.world.canonical(root, here),
            _ => vec![],
        }
    }

    fn builtin(&mut self, b: Builtin) -> Result<Outcome> {
        Ok(match b {
            Builtin::Roots => {
                self.here = None;
                self.entered = None;
                Outcome::Said(self.describe_roots())
            }
            Builtin::Look(target) => {
                let at = match (target.starts_with('/') && target != "/", self.here) {
                    (true, Some(here)) => {
                        let r = self.world.resolve(here, &target).map_err(|s| anyhow!("{target}: {}", s.name()))?;
                        Viewpoint::Instance(r.inst)
                    }
                    (true, None) => bail!("look from a root first; paths resolve in a view"),
                    (false, _) => self.world.find(&target).ok_or_else(|| anyhow!("there's no instance called {target}"))?,
                };
                self.look(at)?;
                Outcome::Said(vec![format!("looking from {}", self.route_line())])
            }
            Builtin::Up => {
                self.up();
                Outcome::Said(vec![if self.here.is_some() { format!("looking from {}", self.route_line()) } else { "at the image".into() }])
            }
            Builtin::View => Outcome::Said(self.describe_view()?),
            Builtin::Cancel(id) => {
                let id = self.cancel(id)?;
                Outcome::Said(vec![format!("cancelling {id}")])
            }
            Builtin::Type(line) => {
                self.type_line(&line)?;
                Outcome::Nothing
            }
            Builtin::Help => Outcome::Said(command::HELP.lines().map(str::to_string).collect()),
            Builtin::Quit => Outcome::Quit,
        })
    }

    /// The route, written out: `web ─/http→ web.http`.
    pub fn route_line(&self) -> String {
        let route = self.route();
        let mut out = String::new();
        for h in &route {
            if let Some(via) = &h.via {
                out.push_str(&format!(" ─{via}{}→ ", if h.read_only { " ro" } else { "" }));
            }
            out.push_str(self.world.name(h.at));
        }
        out
    }

    pub fn describe_roots(&self) -> Vec<String> {
        let w = &self.world;
        let mut lines = vec!["Where to look from:".to_string()];
        for r in w.roots() {
            let held: Vec<&str> = w.descendants(r).into_iter().map(|i| w.instance(i).name.as_str()).collect();
            let what = match r {
                Viewpoint::Root => "the image's own view".to_string(),
                Viewpoint::Instance(i) => format!("{} · {}", w.instance(i).type_name, w.class(i).name()),
            };
            lines.push(format!("  {:<14} {what:<28} holds {}", w.name(r), if held.is_empty() { "nothing".into() } else { held.join(", ") }));
        }
        lines
    }

    pub fn describe_view(&self) -> Result<Vec<String>> {
        let here = self.here.ok_or_else(|| anyhow!("look from somewhere first (:roots)"))?;
        let w = &self.world;
        let mut lines = vec![format!("{}'s view:", w.name(here))];
        for l in w.view(here) {
            let t = l.target.inst;
            let how = match l.how {
                runtime::operator::How::Mount => "mount",
                runtime::operator::How::Bind => "bind",
                runtime::operator::How::Runtime => "+ run time",
            };
            let path = if l.target.path == "/" { String::new() } else { format!(":{}", l.target.path) };
            let ro = if l.target.read_only { " ro" } else { "" };
            lines.push(format!("  {:<14} {how:<10} → {}{path}  {} · {}{ro}", l.at, w.instance(t).name, w.instance(t).type_name, w.class(t).name()));
        }
        if lines.len() == 1 {
            lines.push("  (nothing mounted: it can't make any requests)".into());
        }
        Ok(lines)
    }

    /// The prompt for where the operator looks from: `#` as the operator,
    /// `$` as an object.
    pub fn prompt(&self) -> String {
        match self.here {
            None => "· ".into(),
            Some(at) => format!("{} {} ", self.world.name(at), if self.world.is_operator(at) { '#' } else { '$' }),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.op.send(Command::Stop);
        self.typed.take();
        // The image's thread may be waiting on a socket; don't wait for it.
        drop(self.run.take());
    }
}

/// Lines typed into the image's consoles, read as its stdin.
struct Typed {
    lines: Receiver<String>,
    pending: Vec<u8>,
}

impl Read for Typed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            match self.lines.recv() {
                Ok(line) => self.pending = format!("{line}\n").into_bytes(),
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

/// What the image prints, sent to the shell a line at a time.
struct Printer {
    to: Sender<Raw>,
    line: Vec<u8>,
}

impl Write for Printer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &b in buf {
            if b == b'\n' {
                let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned();
                let _ = self.to.send(Raw::Printed(line));
            } else {
                self.line.push(b);
            }
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
