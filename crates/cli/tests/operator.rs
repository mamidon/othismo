//! The local operator's hold on a running image, which `othismo shell` is
//! built on: requests from a point of view, cancelling, and snapshots.
//! Needs the AssemblyScript modules built first: `task test`.

use othismo::operator::{self, Command, How, Notice, Operator, Snapshot};
use othismo::{status::Status, verb::Verb};
use othismo::payload::Payload;
use othismo::runtime::{Answered, Io, Options, Runtime};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

fn example(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(rel)
}

/// What the image prints, readable from the test's thread.
#[derive(Clone, Default)]
struct Printed(Arc<Mutex<Vec<u8>>>);

impl Printed {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Printed {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `image`, running on a thread of its own under an operator.
struct Operated {
    op: Operator,
    notices: Receiver<Notice>,
    printed: Printed,
    run: Option<JoinHandle<anyhow::Result<()>>>,
    next: u64,
}

impl Operated {
    fn start(image: &str) -> Operated {
        let path = example(image);
        let (op, inbox) = operator::channel();
        let (tx, notices) = mpsc::channel();
        let printed = Printed::default();
        let out = printed.clone();
        let run = std::thread::spawn(move || {
            let mut pack = othismo::pack::load(&path)?;
            othismo::slots::bind(&mut pack.program, &othismo::formats(), &[])?;
            let io = Io { input: Box::new(std::io::empty()), output: Box::new(out) };
            Runtime::new(&pack, io, Options::default())?.run_operated(inbox, move |n| {
                let _ = tx.send(n);
            })
        });
        Operated { op, notices, printed, run: Some(run), next: 0 }
    }

    /// Every answer to `verb path [body]`, made from `from`'s point of view,
    /// up to the last one.
    fn ask(&mut self, from: Option<&str>, verb: Verb, path: &str, body: Option<Payload>) -> Vec<Answered> {
        let from = from.map(|name| self.index(name));
        self.next += 1;
        let id = self.next;
        assert!(self.op.send(Command::Request { id, from, verb, path: path.into(), body }));
        let mut answers = vec![];
        loop {
            let a = self.answer(id);
            let done = a.done;
            answers.push(a);
            if done {
                return answers;
            }
        }
    }

    /// The next answer to request `id`.
    fn answer(&self, id: u64) -> Answered {
        loop {
            match self.notices.recv_timeout(Duration::from_secs(10)).expect("an answer") {
                Notice::Answer { id: got, answer } if got == id => return answer,
                _ => {}
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        assert!(self.op.send(Command::Snapshot));
        loop {
            if let Notice::Snapshot(s) = self.notices.recv_timeout(Duration::from_secs(10)).expect("a snapshot") {
                return s;
            }
        }
    }

    fn index(&self, name: &str) -> usize {
        self.snapshot().instances.iter().position(|i| i.name == name).unwrap_or_else(|| panic!("no instance {name}"))
    }

    fn stop(mut self) -> String {
        self.op.send(Command::Stop);
        self.run.take().unwrap().join().unwrap().unwrap();
        self.printed.text()
    }
}

fn text(s: &str) -> Option<Payload> {
    Some(Payload::new("text/plain".to_string(), s.as_bytes().to_vec()))
}

fn status(answers: &[Answered]) -> Status {
    answers.last().unwrap().status
}

#[test]
fn a_snapshot_shows_every_view_and_how_each_link_was_made() {
    let op = Operated::start("greeter/greeter.oth");
    let s = op.snapshot();
    let names: Vec<&str> = s.instances.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["/dev/cons", "greeter", "shouter"]);
    let link = |links: &[operator::Link], at: &str| links.iter().find(|l| l.at == at).cloned().unwrap();
    assert_eq!(link(&s.root, "/greeter").how, How::Mount);
    assert_eq!(link(&s.root, "/greeter").target.inst, 1);
    let greeter = &s.instances[1];
    assert_eq!(greeter.interface.name, "Greeter");
    assert_eq!(link(&greeter.view, "/dev/cons").how, How::Bind, "passed down from the root's need");
    assert_eq!(link(&greeter.view, "/shouter").how, How::Mount);
    assert!(greeter.served >= 1, "it was configured at startup");
    op.stop();
}

#[test]
fn a_snapshot_shows_what_changed_at_run_time() {
    let op = Operated::start("static/static.test.oth");
    let s = op.snapshot();
    let pages = s.instances.iter().find(|i| i.name == "web.pages").unwrap();
    let statics = pages.exposed.iter().find(|l| l.at == "/static").expect("StaticSite exposes /static when it starts");
    assert_eq!(statics.how, How::Runtime);
    assert!(statics.target.read_only);
    let web = s.instances.iter().find(|i| i.name == "web").unwrap();
    assert!(web.exposed.iter().all(|l| l.how == How::Bind), "an image's exports are passed on, not held");
    op.stop();
}

#[test]
fn an_object_makes_requests_as_itself() {
    let mut op = Operated::start("greeter/greeter.oth");
    assert_eq!(status(&op.ask(Some("greeter"), Verb::Write, "/shouter", text("hi"))), Status::Ok);
    assert_eq!(status(&op.ask(Some("greeter"), Verb::Read, "/shouter", None)), Status::NotSupported, "not among its needs");
    assert_eq!(status(&op.ask(Some("greeter"), Verb::Read, "/nowhere", None)), Status::NotFound);
    assert!(op.stop().contains("HI"));
}

#[test]
fn only_an_image_sends_as_the_operator() {
    let mut op = Operated::start("static/static.test.oth");
    // Configuring a listener is the operator's: an object can't even need it.
    assert_eq!(status(&op.ask(Some("web.http"), Verb::Write, "/net/tcp", text("x"))), Status::NotFound);
    assert_eq!(status(&op.ask(Some("web"), Verb::Write, "/net/tcp", text("x"))), Status::NotSupported, "and once it runs, not even the operator can");
    // From the image that holds it, a host is the operator's to add.
    assert_eq!(status(&op.ask(Some("web"), Verb::Write, "/http/hosts/example.com", text("/www"))), Status::Ok);
    let read = op.ask(Some("web"), Verb::Read, "/http/hosts/example.com", None);
    assert_eq!(read.last().unwrap().body, text("/www"));
    op.stop();
}

#[test]
fn the_root_image_resolves_in_its_own_view() {
    let mut op = Operated::start("greeter/greeter.oth");
    assert_eq!(status(&op.ask(None, Verb::Write, "/dev/cons", text("from the operator"))), Status::Ok);
    assert_eq!(status(&op.ask(None, Verb::Read, "/shouter", None)), Status::NotFound, "the greeter's, not the image's");
    assert!(op.stop().contains("from the operator"));
}

#[test]
fn a_stream_ends_when_the_operator_cancels_it() {
    let mut op = Operated::start("static/static.test.oth");
    op.next += 1;
    let id = op.next;
    let web = op.index("web");
    assert!(op.op.send(Command::Request { id, from: Some(web), verb: Verb::Watch, path: "/dev/cons".into(), body: None }));
    assert!(op.snapshot().instances.iter().any(|i| i.name == "cons" && i.pending == 1), "the watch is open");
    assert!(op.op.send(Command::Cancel { id }));
    let last = op.answer(id);
    assert!(last.done && last.status == Status::Ok);
    assert!(op.snapshot().instances.iter().all(|i| i.name != "cons" || i.pending == 0), "and closed");
    op.stop();
}

#[test]
fn the_operator_can_look_inside_any_served_tree() {
    let op = Operated::start("static/static.test.oth");
    let files = op.index("web.files");
    let look = |verb: Verb, path: &str| {
        assert!(op.op.send(Command::Look { id: 99, inst: files, verb, path: path.into() }));
        let mut answers = vec![];
        loop {
            let a = op.answer(99);
            let done = a.done;
            answers.push(a);
            if done {
                return answers;
            }
        }
    };
    let listed = look(Verb::List, "/");
    assert!(listed.len() > 2, "what web.files serves, though nothing looks from a view that has it: {listed:?}");
    assert_eq!(status(&look(Verb::Write, "/x")), Status::Invalid, "looking changes nothing");
    assert_eq!(status(&look(Verb::List, "/../x")), Status::Invalid);
    op.stop();
}
