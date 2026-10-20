//! Runtime behaviour, exercised with hand-written WAT modules so these tests
//! don't need the AssemblyScript toolchain.

mod common;

use bson::doc;
use common::{Fixture, run_image};
use real_talk::runtime::Options;
use real_talk::wire::Message;

const MESSAGES: &str = "
export class Line { text: string }
export class Echoed { text: string }
export class Ping {}
";

/// What a scripted module does on its n-th delivery.
enum Act {
    Emit(Message),
    /// Reply to whoever sent the message being handled.
    Reply(Message),
    /// Reply using a made-up handle.
    ReplyTo(u32, Message),
}

/// A module performing `acts[n]` on its n-th delivery, then nothing.
fn script_wat(acts: &[Act]) -> String {
    let mut data = String::new();
    let mut steps = String::new();
    let mut offset = 0;
    for (n, act) in acts.iter().enumerate() {
        let (Act::Emit(m) | Act::Reply(m) | Act::ReplyTo(_, m)) = act;
        let bytes = m.encode();
        let escaped: String = bytes.iter().map(|b| format!("\\{b:02x}")).collect();
        data.push_str(&format!("(data (i32.const {offset}) \"{escaped}\")\n"));
        let call = match act {
            Act::Emit(_) => format!("(call $emit (i32.const {offset}) (i32.const {}))", bytes.len()),
            Act::Reply(_) => format!("(call $reply (local.get 0) (i32.const {offset}) (i32.const {}))", bytes.len()),
            Act::ReplyTo(h, _) => format!("(call $reply (i32.const {h}) (i32.const {offset}) (i32.const {}))", bytes.len()),
        };
        steps.push_str(&format!("(if (i32.eq (global.get $n) (i32.const {n})) (then {call}))\n"));
        offset += bytes.len() + 16;
    }
    format!(
        r#"(module
          (import "real_talk" "emit" (func $emit (param i32 i32)))
          (import "real_talk" "reply" (func $reply (param i32 i32 i32)))
          (memory (export "memory") 1)
          (global $n (mut i32) (i32.const 0))
          {data}
          (func (export "rt_inbox") (param i32) (result i32) (i32.const 60000))
          (func (export "rt_deliver") (param i32)
            {steps}
            (global.set $n (i32.add (global.get $n) (i32.const 1)))))"#
    )
}

fn echoed(text: &str) -> Message {
    Message { type_name: "Echoed".into(), fields: doc! { "text": text } }
}

fn line(text: &str) -> Message {
    Message { type_name: "Line".into(), fields: doc! { "text": text } }
}

fn ping() -> Message {
    Message { type_name: "Ping".into(), fields: doc! {} }
}

/// A `Sender` protocol (in Ping, emits Echoed) poked by one startup Ping.
fn sender_fixture(name: &str, wat: &str, routes: &str) -> Fixture {
    let f = Fixture::new(name);
    f.file("messages.ts", MESSAGES).file("scripts/Sender.wasm", wat).file(
        "test.img",
        format!(
            r#"import {{ Line, Echoed, Ping }} from "./messages"
            protocol Sender {{ in Ping  emits Echoed }}
            instance s : Sender {{ send Ping }}
            instance a : Environment.Stdout
            instance b : Environment.Stdout
            {routes}"#
        ),
    );
    f
}

fn run(f: &Fixture) -> anyhow::Result<String> {
    run_image(&f.image(), "", Options::default())
}

fn run_err(f: &Fixture) -> String {
    format!("{:#}", run(f).unwrap_err())
}

#[test]
fn startup_send_triggers_emit_that_fans_out() {
    let f = sender_fixture("fanout", &script_wat(&[Act::Emit(echoed("hi"))]), "s.Echoed { a, b }");
    assert_eq!(run(&f).unwrap(), "hi\nhi\n");
}

#[test]
fn unrouted_emits_are_dropped() {
    let f = sender_fixture("unrouted", &script_wat(&[Act::Emit(echoed("hi"))]), "");
    assert_eq!(run(&f).unwrap(), "");
}

#[test]
fn undeclared_emit_is_refused() {
    let f = sender_fixture("wrongtype", &script_wat(&[Act::Emit(line("x"))]), "s.Echoed { a }");
    assert!(run_err(&f).contains("instance `s` emitted `Line`, but its protocol only emits Echoed"));
}

#[test]
fn shape_checking_is_opt_in() {
    let bad = Message { type_name: "Echoed".into(), fields: doc! { "text": 5 } };
    let f = sender_fixture("shape", &script_wat(&[Act::Emit(bad)]), "s.Echoed { a }");
    assert_eq!(run(&f).unwrap(), "Echoed {\"text\": 5}\n");
    let err = format!("{:#}", run_image(&f.image(), "", Options { check_shapes: true }).unwrap_err());
    assert!(err.contains("`Echoed.text` should be string"), "{err}");
}

/// `asker` gets a startup Ping and emits Ping to `answerer`; whatever comes
/// back triggers asker's second act, an Echoed routed to stdout.
fn ask_fixture(name: &str, answerer_wat: &str) -> Fixture {
    let f = Fixture::new(name);
    let asker = script_wat(&[Act::Emit(ping()), Act::Emit(echoed("got a reply"))]);
    f.file("messages.ts", MESSAGES).file("scripts/Asker.wasm", asker).file("scripts/Answerer.wasm", answerer_wat).file(
        "test.img",
        r#"import { Line, Echoed, Ping } from "./messages"
        protocol Asker { in Ping, Line  emits Ping, Echoed }
        protocol Answerer { in Ping  replies Line }
        instance asker : Asker { send Ping }
        instance answerer : Answerer
        instance out : Environment.Stdout
        asker.Ping { answerer }
        asker.Echoed { out }"#,
    );
    f
}

#[test]
fn replies_go_back_to_the_sender() {
    let f = ask_fixture("reply", &script_wat(&[Act::Reply(line("pong"))]));
    assert_eq!(run(&f).unwrap(), "got a reply\n");
}

#[test]
fn reply_handles_cannot_be_invented() {
    let f = ask_fixture("badhandle", &script_wat(&[Act::ReplyTo(7, line("pong"))]));
    assert!(run_err(&f).contains("instance `answerer` replied with unknown handle 7"));
}

#[test]
fn undeclared_reply_is_refused() {
    let f = ask_fixture("badreply", &script_wat(&[Act::Reply(echoed("pong"))]));
    assert!(run_err(&f).contains("instance `answerer` replied `Echoed`, but its protocol only replies Line"));
}

#[test]
fn replies_to_startup_sends_go_nowhere() {
    let f = sender_fixture("replyimage", &script_wat(&[Act::Reply(echoed("x"))]), "s.Echoed { a }");
    let img = std::fs::read_to_string(f.image()).unwrap();
    f.file("test.img", img.replace("emits Echoed", "emits Echoed  replies Echoed"));
    assert_eq!(run(&f).unwrap(), "");
}

#[test]
fn files_reply_to_whoever_asked() {
    let f = Fixture::new("files");
    let read = Message { type_name: "ReadFile".into(), fields: doc! { "tag": 1i64, "path": "/hello.txt" } };
    let asker = script_wat(&[Act::Emit(read), Act::Emit(echoed("got the file"))]);
    std::fs::create_dir_all(f.dir.join("public")).unwrap();
    f.file("public/hello.txt", "hi").file("messages.ts", MESSAGES).file("scripts/Asker.wasm", asker).file(
        "test.img",
        r#"import { Echoed, Ping } from "./messages"
        import { FilesRoot, ReadFile, FileContents, FileError } from "real-talk/env"
        protocol Asker { in Ping, FileContents, FileError  emits ReadFile, Echoed }
        instance asker : Asker { send Ping }
        instance fs : Environment.Files { send FilesRoot { path: "public" } }
        instance out : Environment.Stdout
        asker.ReadFile { fs }
        asker.Echoed { out }"#,
    );
    assert_eq!(run(&f).unwrap(), "got the file\n");
}

#[test]
fn modules_cannot_import_anything_else() {
    let wat = r#"(module
      (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
      (memory (export "memory") 1)
      (func (export "rt_inbox") (param i32) (result i32) (i32.const 0))
      (func (export "rt_deliver") (param i32)))"#;
    let f = sender_fixture("wasi", wat, "");
    assert!(run_err(&f).contains("imports `wasi_snapshot_preview1.fd_write`, which the runtime does not provide"));
}

#[test]
fn abort_fails_the_run_naming_the_instance() {
    // An AssemblyScript string: u32 byte length at ptr-4, then UTF-16LE.
    let wat = r#"(module
      (import "env" "abort" (func $abort (param i32 i32 i32 i32)))
      (memory (export "memory") 1)
      (data (i32.const 12) "\08\00\00\00b\00o\00o\00m\00")
      (func (export "rt_inbox") (param i32) (result i32) (i32.const 4096))
      (func (export "rt_deliver") (param i32) (call $abort (i32.const 16) (i32.const 0) (i32.const 7) (i32.const 9))))"#;
    let f = sender_fixture("abort", wat, "");
    let err = run_err(&f);
    assert!(err.contains("instance `s` failed handling `Ping`"), "{err}");
    assert!(err.contains("abort: boom"), "{err}");
}

#[test]
fn missing_module_is_reported() {
    let f = Fixture::new("missing");
    f.file("messages.ts", MESSAGES).file(
        "test.img",
        r#"import { Ping } from "./messages"
        protocol Ghost { in Ping }
        instance g : Ghost"#,
    );
    assert!(run_err(&f).contains("did you compile Ghost.ts?"));
}

#[test]
fn stdin_lines_flow_to_stdout() {
    let f = Fixture::new("cat");
    f.file("messages.ts", MESSAGES).file(
        "test.img",
        r#"import { Line } from "./messages"
        instance i : Environment.Stdin
        instance o : Environment.Stdout
        i.Line { o }"#,
    );
    assert_eq!(run_image(&f.image(), "a\r\nb\n\nc", Options::default()).unwrap(), "a\nb\n\nc\n");
}
