//! Runtime behaviour, exercised with generated WAT modules so these tests
//! don't need the AssemblyScript toolchain.

mod common;

use bson::doc;
use common::{Fixture, run_image};
use othismo::ns::{Status, Verb};
use othismo::runtime::Options;
use othismo::wire::Message;

const MESSAGES: &str = "export class Ping {}\n";

/// One thing a scripted module does.
enum Act {
    /// Makes a request; with `expect`, the ticket is kept for `Cancel`.
    Call { verb: Verb, path: &'static str, body: Option<Message>, expect: bool },
    /// Replies to the request being served.
    Reply { status: Status, body: Option<Message>, done: bool },
    /// Replies to a made-up request id.
    ReplyTo(u32),
    /// Forwards the request being served.
    Forward(&'static str),
    /// Cancels the last call made with `expect`.
    Cancel,
    Bind { src: &'static str, at: &'static str, read_only: bool },
    Expose { at: &'static str, src: &'static str, read_only: bool },
    Unmount(&'static str),
    /// Writes the last status (an answer's or a bind's) to `path` as Text,
    /// e.g. "not_found", or "ok (more)" for a reply that isn't the last.
    Report(&'static str),
    /// Writes the body being handled to `path`.
    Echo(&'static str),
    /// `Echo` if there is a body, otherwise `Report`.
    Show(&'static str),
}

/// What a module does in each entry point.
enum Steps {
    /// The same acts on every call.
    Each(Vec<Act>),
    /// `.0[n]` on the n-th call, then nothing.
    Nth(Vec<Vec<Act>>),
}

#[derive(Default)]
struct Script {
    start: Vec<Act>,
    serve: Option<Steps>,
    answer: Option<Steps>,
}

fn text(s: &str) -> Message {
    Message { type_name: "Text".into(), fields: doc! { "text": s } }
}

fn call(verb: Verb, path: &'static str) -> Act {
    Act::Call { verb, path, body: None, expect: true }
}

fn write(path: &'static str, body: Message) -> Act {
    Act::Call { verb: Verb::Write, path, body: Some(body), expect: false }
}

fn reply(body: Message) -> Act {
    Act::Reply { status: Status::Ok, body: Some(body), done: true }
}

const INBOX: usize = 65536;
const STATUSES: [Status; 10] = [
    Status::Ok,
    Status::NotFound,
    Status::ReadOnly,
    Status::NotSupported,
    Status::IsDirectory,
    Status::NotDirectory,
    Status::Exists,
    Status::Invalid,
    Status::Io,
    Status::Forbidden,
];

/// Builds the data segment and code for a script.
struct Wat {
    data: Vec<u8>,
}

impl Wat {
    /// Places `bytes` in the data segment; returns `(offset, len)`.
    fn put(&mut self, bytes: &[u8]) -> (usize, usize) {
        let at = 1024 + self.data.len();
        self.data.extend_from_slice(bytes);
        (at, bytes.len())
    }

    fn body(&mut self, body: &Option<Message>) -> (usize, usize) {
        body.as_ref().map(|m| self.put(&m.encode())).unwrap_or((0, 0))
    }

    fn acts(&mut self, acts: &[Act]) -> String {
        acts.iter().map(|a| self.act(a)).collect::<Vec<_>>().join("\n")
    }

    fn act(&mut self, act: &Act) -> String {
        match act {
            Act::Call { verb, path, body, expect } => {
                let (p, pl) = self.put(path.as_bytes());
                let (b, bl) = self.body(body);
                let call = wcall("call", &[k(*verb as usize), k(p), k(pl), k(b), k(bl), k(*expect as usize)]);
                if *expect { format!("(global.set $ticket {call})") } else { format!("(drop {call})") }
            }
            Act::Reply { status, body, done } => {
                let (b, bl) = self.body(body);
                wcall("reply", &[g("req"), k(*status as usize), k(b), k(bl), k(*done as usize)])
            }
            Act::ReplyTo(req) => wcall("reply", &[k(*req as usize), k(0), k(0), k(0), k(1)]),
            Act::Forward(path) => {
                let (p, pl) = self.put(path.as_bytes());
                wcall("forward", &[g("req"), k(p), k(pl)])
            }
            Act::Cancel => wcall("cancel", &[g("ticket")]),
            Act::Bind { src, at, read_only } | Act::Expose { at, src, read_only } => {
                let (s, sl) = self.put(src.as_bytes());
                let (a, al) = self.put(at.as_bytes());
                let call = match act {
                    Act::Bind { .. } => wcall("bind", &[k(s), k(sl), k(a), k(al), k(*read_only as usize)]),
                    _ => wcall("expose", &[k(a), k(al), k(s), k(sl), k(*read_only as usize)]),
                };
                format!("(global.set $status {call}) (global.set $done (i32.const 1))")
            }
            Act::Unmount(at) => {
                let (a, al) = self.put(at.as_bytes());
                let call = wcall("unmount", &[k(a), k(al)]);
                format!("(global.set $status {call}) (global.set $done (i32.const 1))")
            }
            Act::Report(path) => {
                let mut code = String::new();
                for status in STATUSES {
                    for done in [true, false] {
                        let msg = if done { format!("{status}") } else { format!("{status} (more)") };
                        let write = self.act(&write(path, text(&msg)));
                        let is = |var: &str, v: usize| format!("(i32.eq {} {})", g(var), k(v));
                        let cond = format!("(i32.and {} {})", is("status", status as usize), is("done", done as usize));
                        code.push_str(&format!("(if {cond} (then {write}))\n"));
                    }
                }
                code
            }
            Act::Echo(path) => {
                let (p, pl) = self.put(path.as_bytes());
                let call = wcall("call", &[k(Verb::Write as usize), k(p), k(pl), g("bptr"), g("blen"), k(0)]);
                format!("(drop {call})")
            }
            Act::Show(path) => {
                let (echo, report) = (self.act(&Act::Echo(path)), self.act(&Act::Report(path)));
                format!("(if (i32.gt_u (global.get $blen) (i32.const 0)) (then {echo}) (else {report}))")
            }
        }
    }

    fn steps(&mut self, steps: &Option<Steps>, counter: &str) -> String {
        match steps {
            None => String::new(),
            Some(Steps::Each(acts)) => self.acts(acts),
            Some(Steps::Nth(nth)) => nth
                .iter()
                .enumerate()
                .map(|(n, acts)| format!("(if (i32.eq (global.get {counter}) (i32.const {n})) (then {}))", self.acts(acts)))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// `(call $name args...)`
fn wcall(name: &str, args: &[String]) -> String {
    format!("(call ${name} {})", args.join(" "))
}

fn k(v: usize) -> String {
    format!("(i32.const {v})")
}

fn g(name: &str) -> String {
    format!("(global.get ${name})")
}

fn script_wat(s: &Script) -> String {
    let mut w = Wat { data: vec![] };
    let start = w.acts(&s.start);
    let serve = w.steps(&s.serve, "$sn");
    let answer = w.steps(&s.answer, "$an");
    let data: String = w.data.iter().map(|b| format!("\\{b:02x}")).collect();
    format!(
        r#"(module
          (import "othismo" "call" (func $call (param i32 i32 i32 i32 i32 i32) (result i32)))
          (import "othismo" "reply" (func $reply (param i32 i32 i32 i32 i32)))
          (import "othismo" "forward" (func $forward (param i32 i32 i32)))
          (import "othismo" "cancel" (func $cancel (param i32)))
          (import "othismo" "bind" (func $bind (param i32 i32 i32 i32 i32) (result i32)))
          (import "othismo" "unmount" (func $unmount (param i32 i32) (result i32)))
          (import "othismo" "expose" (func $expose (param i32 i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 3)
          (data (i32.const 1024) "{data}")
          (global $req (mut i32) (i32.const 0))
          (global $ticket (mut i32) (i32.const 0))
          (global $status (mut i32) (i32.const 0))
          (global $done (mut i32) (i32.const 0))
          (global $bptr (mut i32) (i32.const 0))
          (global $blen (mut i32) (i32.const 0))
          (global $sn (mut i32) (i32.const 0))
          (global $an (mut i32) (i32.const 0))
          (func (export "rt_inbox") (param i32) (result i32) (i32.const {INBOX}))
          (func (export "rt_start") {start})
          (func (export "rt_serve") (param i32 i32 i32 i32)
            (global.set $req (local.get 0))
            (global.set $bptr (i32.add (i32.const {INBOX}) (local.get 2)))
            (global.set $blen (local.get 3))
            {serve}
            (global.set $sn (i32.add (global.get $sn) (i32.const 1))))
          (func (export "rt_answer") (param i32 i32 i32 i32)
            (global.set $status (local.get 1))
            (global.set $done (local.get 3))
            (global.set $bptr (i32.const {INBOX}))
            (global.set $blen (local.get 2))
            {answer}
            (global.set $an (i32.add (global.get $an) (i32.const 1)))))"#
    )
}

/// An image with a Console called `cons`, the given Wasm instances
/// (`name`, script, members) and any extra declarations.
fn fixture(name: &str, objects: &[(&str, &Script, &str)], extra: &str) -> Fixture {
    let f = Fixture::new(name);
    let mut img = format!("import {{ Ping }} from \"./messages\"\ninstance cons : Environment.Console\n{extra}\n");
    for (obj, script, members) in objects {
        let ty = format!("{}{}", obj[..1].to_uppercase(), &obj[1..]);
        f.file(&format!("scripts/{ty}.wasm"), script_wat(script));
        img.push_str(&format!("instance {obj} : {ty} {{ {members} }}\n"));
    }
    f.file("messages.ts", MESSAGES).file("test.img", img);
    f
}

fn run(f: &Fixture) -> anyhow::Result<String> {
    run_image(&f.image(), "", Options::default())
}

fn run_err(f: &Fixture) -> String {
    format!("{:#}", run(f).unwrap_err())
}

#[test]
fn startup_requests_reach_the_instance_which_uses_its_view() {
    let a = Script { serve: Some(Steps::Each(vec![write("/out", text("hi"))])), ..Default::default() };
    let f = fixture("startup", &[("a", &a, "mount cons /out  invoke / Ping  invoke /again Ping")], "");
    assert_eq!(run(&f).unwrap(), "hi\nhi\n");
}

#[test]
fn paths_outside_the_view_are_not_found() {
    let a = Script {
        start: vec![call(Verb::Read, "/secret"), call(Verb::Read, "/out/../x"), call(Verb::Read, "/out")],
        answer: Some(Steps::Each(vec![Act::Report("/out")])),
        ..Default::default()
    };
    let b = Script { start: vec![write("/out", text("b has no view"))], ..Default::default() };
    let f = fixture("unseen", &[("a", &a, "mount cons /out"), ("b", &b, "")], "");
    assert_eq!(run(&f).unwrap(), "not_found\ninvalid\nnot_supported\n");
}

#[test]
fn read_only_mounts_refuse_unsafe_verbs() {
    let a = Script {
        start: vec![Act::Call { verb: Verb::Write, path: "/b/x", body: Some(text("x")), expect: true }, call(Verb::Read, "/b/x")],
        answer: Some(Steps::Each(vec![Act::Report("/out")])),
        ..Default::default()
    };
    let b = Script {
        serve: Some(Steps::Each(vec![write("/out", text("b served")), Act::Reply { status: Status::Ok, body: None, done: true }])),
        ..Default::default()
    };
    let f = fixture("readonly", &[("a", &a, "mount b /b readonly  mount cons /out"), ("b", &b, "mount cons /out")], "");
    assert_eq!(run(&f).unwrap(), "read_only\nb served\nok\n", "b never sees the write");
}

#[test]
fn only_the_image_controls() {
    let a = Script {
        start: vec![Act::Call { verb: Verb::Control, path: "/b/x", body: Some(text("x")), expect: true }],
        answer: Some(Steps::Each(vec![Act::Report("/out")])),
        ..Default::default()
    };
    let b = Script {
        serve: Some(Steps::Each(vec![write("/out", text("b served")), Act::Reply { status: Status::Ok, body: None, done: true }])),
        ..Default::default()
    };
    let f = fixture("control", &[("a", &a, "mount b /b  mount cons /out"), ("b", &b, "mount cons /out  control /y Ping")], "");
    assert_eq!(run(&f).unwrap(), "b served\nforbidden\n", "b serves the image's control, never a's");
}

#[test]
fn replies_stream_until_done() {
    let a = Script {
        start: vec![call(Verb::List, "/b")],
        answer: Some(Steps::Each(vec![Act::Report("/out")])),
        ..Default::default()
    };
    let more = |s| Act::Reply { status: Status::Ok, body: Some(text(s)), done: false };
    let b = Script {
        serve: Some(Steps::Each(vec![more("1"), more("2"), Act::Reply { status: Status::Ok, body: None, done: true }])),
        ..Default::default()
    };
    let f = fixture("stream", &[("a", &a, "mount b /b  mount cons /out"), ("b", &b, "")], "");
    assert_eq!(run(&f).unwrap(), "ok (more)\nok (more)\nok\n");
}

#[test]
fn forward_hands_the_request_on_in_the_servers_view() {
    let a = Script {
        start: vec![call(Verb::Read, "/b/one"), call(Verb::Read, "/b/two")],
        answer: Some(Steps::Each(vec![Act::Show("/out")])),
        ..Default::default()
    };
    let b = Script {
        serve: Some(Steps::Nth(vec![vec![Act::Forward("/c/deep")], vec![Act::Forward("/nowhere")]])),
        ..Default::default()
    };
    let c = Script { serve: Some(Steps::Each(vec![reply(text("from c"))])), ..Default::default() };
    let f = fixture("forward", &[("a", &a, "mount b /b  mount cons /out"), ("b", &b, "mount c /c"), ("c", &c, "")], "");
    // The failed forward is answered at once; c's answer takes a round trip.
    assert_eq!(run(&f).unwrap(), "not_found\nfrom c\n");
}

#[test]
fn exposures_route_past_the_server() {
    // b exposes c at /pub; a only sees b. b's start runs before a's (image order).
    let b = Script {
        start: vec![Act::Expose { at: "/pub", src: "/c", read_only: true }, Act::Report("/out")],
        serve: Some(Steps::Each(vec![reply(text("from b"))])),
        ..Default::default()
    };
    let c = Script { serve: Some(Steps::Each(vec![reply(text("from c"))])), ..Default::default() };
    let a = Script {
        start: vec![
            call(Verb::Read, "/b/pub/x"),
            call(Verb::Read, "/b/public"),
            Act::Call { verb: Verb::Write, path: "/b/pub/x", body: Some(text("x")), expect: true },
        ],
        answer: Some(Steps::Each(vec![Act::Show("/out")])),
        ..Default::default()
    };
    let f = fixture(
        "expose",
        &[("b", &b, "mount c /c  mount cons /out"), ("c", &c, ""), ("a", &a, "mount b /b  mount cons /out")],
        "",
    );
    // b's expose reports Ok; the refused write is answered before the reads come back.
    assert_eq!(run(&f).unwrap(), "ok\nread_only\nfrom c\nfrom b\n");
}

#[test]
fn bind_and_unmount_rearrange_only_the_own_view() {
    let a = Script {
        start: vec![
            Act::Bind { src: "/b/sub", at: "/alias", read_only: false },
            Act::Report("/out"),
            Act::Bind { src: "/nothing", at: "/x", read_only: false },
            Act::Report("/out"),
            Act::Unmount("/b"),
            Act::Report("/out"),
            Act::Unmount("/b"),
            Act::Report("/out"),
            call(Verb::Read, "/alias/y"),
            call(Verb::Read, "/b/y"),
        ],
        answer: Some(Steps::Each(vec![Act::Show("/out")])),
        ..Default::default()
    };
    let b = Script { serve: Some(Steps::Each(vec![reply(text("from b"))])), ..Default::default() };
    let f = fixture("bind", &[("a", &a, "mount b /b  mount cons /out"), ("b", &b, "")], "");
    assert_eq!(run(&f).unwrap(), "ok\nnot_found\nok\nnot_found\nnot_found\nfrom b\n");
}

#[test]
fn console_lines_stream_to_watchers() {
    let a = Script {
        start: vec![call(Verb::Watch, "/cons")],
        answer: Some(Steps::Each(vec![Act::Echo("/cons")])),
        ..Default::default()
    };
    let f = fixture("cat", &[("a", &a, "mount cons /cons")], "");
    // The end of the stream has no body; echoing it is a Write without one, which Console refuses.
    assert_eq!(run_image(&f.image(), "a\r\nb\n\nc", Options::default()).unwrap(), "a\nb\n\nc\n");
}

#[test]
fn cancel_stops_a_stream() {
    let a = Script {
        start: vec![call(Verb::Watch, "/cons")],
        answer: Some(Steps::Nth(vec![vec![Act::Echo("/cons"), Act::Cancel]])),
        ..Default::default()
    };
    let f = fixture("cancel", &[("a", &a, "mount cons /cons")], "");
    assert_eq!(run_image(&f.image(), "a\nb\nc\n", Options::default()).unwrap(), "a\n");
}

/// Reads, stats, lists and tries to remove through `/f`.
fn files_script() -> Script {
    Script {
        start: vec![
            call(Verb::Read, "/f/hello.txt"),
            call(Verb::Read, "/f/missing"),
            call(Verb::Read, "/f/sub"),
            call(Verb::Stat, "/f/hello.txt"),
            call(Verb::List, "/f/"),
            Act::Call { verb: Verb::Remove, path: "/f/hello.txt", body: None, expect: true },
        ],
        answer: Some(Steps::Each(vec![Act::Show("/out")])),
        ..Default::default()
    }
}

const FILES_OUTPUT: &str = "Data {\"mime\": \"text/plain; charset=utf-8\", \"data\": \"hi\"}\n\
    not_found\n\
    is_directory\n\
    Info {\"kind\": \"file\", \"size\": 2, \"mime\": \"text/plain; charset=utf-8\"}\n\
    Entry {\"name\": \"hello.txt\", \"kind\": \"file\"}\n\
    Entry {\"name\": \"sub\", \"kind\": \"dir\"}\n\
    ok\n\
    read_only\n";

#[test]
fn files_read_stat_and_list() {
    let files = "instance files : Environment.Files { control / FilesRoot { path: \"public\" } }";
    let f = fixture("files", &[("a", &files_script(), "mount files /f  mount cons /out")], files);
    std::fs::create_dir_all(f.dir.join("public/sub")).unwrap();
    f.file("public/hello.txt", "hi");
    assert_eq!(run(&f).unwrap(), FILES_OUTPUT);
}

#[test]
fn packed_files_answer_like_files() {
    let files = "instance files : Packed.Files { include \"public/\" }";
    let f = fixture("packed", &[("a", &files_script(), "mount files /f  mount cons /out")], files);
    std::fs::create_dir_all(f.dir.join("public/sub")).unwrap();
    f.file("public/hello.txt", "hi");
    let pack = othismo::pack::load(&f.image()).unwrap();
    // Nothing on disk is read once the image is built.
    std::fs::remove_dir_all(f.dir.join("public")).unwrap();
    let out = common::Captured::default();
    let io = othismo::runtime::Io { input: Box::new(std::io::empty()), output: Box::new(out.clone()) };
    othismo::runtime::run(&pack, io, Options::default()).unwrap();
    assert_eq!(out.text(), FILES_OUTPUT);
}

#[test]
fn unknown_request_ids_stop_the_run() {
    let a = Script { serve: Some(Steps::Each(vec![Act::ReplyTo(7)])), ..Default::default() };
    let f = fixture("badreq", &[("a", &a, "invoke / Ping")], "");
    let e = run_err(&f);
    assert!(e.contains("instance `a` replied to unknown request 7"), "{e}");
}

#[test]
fn shape_checking_is_opt_in() {
    let bad = Message { type_name: "Text".into(), fields: doc! { "text": 5 } };
    let a = Script { start: vec![write("/out", bad)], ..Default::default() };
    let f = fixture("shape", &[("a", &a, "mount cons /out")], "");
    assert_eq!(run(&f).unwrap(), "Text {\"text\": 5}\n");
    let err = format!("{:#}", run_image(&f.image(), "", Options { check_shapes: true }).unwrap_err());
    assert!(err.contains("`Text.text` should be string"), "{err}");

    let odd = Message { type_name: "Mystery".into(), fields: doc! {} };
    let a = Script { start: vec![write("/out", odd)], ..Default::default() };
    let f = fixture("undeclared", &[("a", &a, "mount cons /out")], "");
    let err = format!("{:#}", run_image(&f.image(), "", Options { check_shapes: true }).unwrap_err());
    assert!(err.contains("sent `Mystery`, which isn't declared"), "{err}");
}

#[test]
fn modules_cannot_import_anything_else() {
    let wat = r#"(module
      (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
      (memory (export "memory") 1))"#;
    let f = fixture("wasi", &[], "instance s : S");
    f.file("scripts/S.wasm", wat);
    assert!(run_err(&f).contains("module S imports `wasi_snapshot_preview1.fd_write`, which the runtime does not provide"));
}

#[test]
fn modules_must_export_the_entry_points() {
    let f = fixture("exports", &[], "instance s : S");
    f.file(
        "scripts/S.wasm",
        r#"(module (memory (export "memory") 1) (func (export "rt_inbox") (param i32) (result i32) (i32.const 0)))"#,
    );
    assert!(run_err(&f).contains("module must export rt_serve(i32, i32, i32, i32)"));
}

#[test]
fn abort_fails_the_run_naming_the_instance() {
    // An AssemblyScript string: u32 byte length at ptr-4, then UTF-16LE.
    let wat = r#"(module
      (import "env" "abort" (func $abort (param i32 i32 i32 i32)))
      (memory (export "memory") 1)
      (data (i32.const 12) "\08\00\00\00b\00o\00o\00m\00")
      (func (export "rt_inbox") (param i32) (result i32) (i32.const 4096))
      (func (export "rt_serve") (param i32 i32 i32 i32))
      (func (export "rt_answer") (param i32 i32 i32 i32))
      (func (export "rt_start") (call $abort (i32.const 16) (i32.const 0) (i32.const 7) (i32.const 9))))"#;
    let f = fixture("abort", &[], "instance s : S");
    f.file("scripts/S.wasm", wat);
    let err = run_err(&f);
    assert!(err.contains("instance `s` failed starting"), "{err}");
    assert!(err.contains("abort: boom"), "{err}");
}

#[test]
fn missing_module_is_reported() {
    let f = fixture("missing", &[], "instance g : Ghost");
    assert!(run_err(&f).contains("did you compile Ghost.ts?"));
}
