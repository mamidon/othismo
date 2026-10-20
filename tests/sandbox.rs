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

/// A module that, on every delivery, sends `msg` on out port `port`.
fn sender_wat(port: u32, msg: &Message) -> String {
    let bytes = msg.encode();
    let data: String = bytes.iter().map(|b| format!("\\{b:02x}")).collect();
    format!(
        r#"(module
          (import "real_talk" "send" (func $send (param i32 i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{data}")
          (func (export "rt_inbox") (param i32) (result i32) (i32.const 4096))
          (func (export "rt_deliver") (call $send (i32.const {port}) (i32.const 0) (i32.const {len}))))"#,
        len = bytes.len()
    )
}

fn echoed(text: &str) -> Message {
    Message { type_name: "Echoed".into(), fields: doc! { "text": text } }
}

/// A `Sender` protocol (in Ping, out out: Echoed) poked by one startup Ping.
fn sender_fixture(name: &str, wat: &str, wiring: &str) -> Fixture {
    let f = Fixture::new(name);
    f.file("messages.ts", MESSAGES).file("scripts/Sender.wasm", wat).file(
        "test.img",
        format!(
            r#"import {{ Line, Echoed, Ping }} from "./messages"
            protocol Sender {{ in Ping  out out: Echoed }}
            instance s : Sender {{ send Ping  {wiring} }}
            instance a : Environment.Stdout
            instance b : Environment.Stdout"#
        ),
    );
    f
}

#[test]
fn startup_send_routes_and_fans_out() {
    let f = sender_fixture("fanout", &sender_wat(0, &echoed("hi")), "out -> a, b");
    assert_eq!(run_image(&f.image(), "", Options::default()).unwrap(), "hi\nhi\n");
}

#[test]
fn undeclared_message_type_on_port_is_refused() {
    let line = Message { type_name: "Line".into(), fields: doc! { "text": "x" } };
    let f = sender_fixture("wrongtype", &sender_wat(0, &line), "out -> a");
    let err = format!("{:#}", run_image(&f.image(), "", Options::default()).unwrap_err());
    assert!(err.contains("instance `s` sent `Line` on port `out`, which only carries Echoed"), "{err}");
}

#[test]
fn undeclared_port_is_refused() {
    let f = sender_fixture("badport", &sender_wat(1, &echoed("x")), "out -> a");
    let err = format!("{:#}", run_image(&f.image(), "", Options::default()).unwrap_err());
    assert!(err.contains("sent on port #1"), "{err}");
}

#[test]
fn shape_checking_is_opt_in() {
    let bad = Message { type_name: "Echoed".into(), fields: doc! { "text": 5 } };
    let f = sender_fixture("shape", &sender_wat(0, &bad), "out -> a");
    assert_eq!(run_image(&f.image(), "", Options::default()).unwrap(), "Echoed {\"text\": 5}\n");
    let err = format!("{:#}", run_image(&f.image(), "", Options { check_shapes: true }).unwrap_err());
    assert!(err.contains("`Echoed.text` should be string"), "{err}");
}

#[test]
fn modules_cannot_import_anything_else() {
    let wat = r#"(module
      (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
      (memory (export "memory") 1)
      (func (export "rt_inbox") (param i32) (result i32) (i32.const 0))
      (func (export "rt_deliver")))"#;
    let f = sender_fixture("wasi", wat, "out -> a");
    let err = format!("{:#}", run_image(&f.image(), "", Options::default()).unwrap_err());
    assert!(err.contains("imports `wasi_snapshot_preview1.fd_write`, which the runtime does not provide"), "{err}");
}

#[test]
fn abort_fails_the_run_naming_the_instance() {
    // An AssemblyScript string: u32 byte length at ptr-4, then UTF-16LE.
    let wat = r#"(module
      (import "env" "abort" (func $abort (param i32 i32 i32 i32)))
      (memory (export "memory") 1)
      (data (i32.const 12) "\08\00\00\00b\00o\00o\00m\00")
      (func (export "rt_inbox") (param i32) (result i32) (i32.const 4096))
      (func (export "rt_deliver") (call $abort (i32.const 16) (i32.const 0) (i32.const 7) (i32.const 9))))"#;
    let f = sender_fixture("abort", wat, "out -> a");
    let err = format!("{:#}", run_image(&f.image(), "", Options::default()).unwrap_err());
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
    let err = format!("{:#}", run_image(&f.image(), "", Options::default()).unwrap_err());
    assert!(err.contains("did you compile Ghost.ts?"), "{err}");
}

#[test]
fn stdin_lines_flow_to_stdout() {
    let f = Fixture::new("cat");
    f.file("messages.ts", MESSAGES).file(
        "test.img",
        r#"import { Line } from "./messages"
        instance i : Environment.Stdin { out -> o }
        instance o : Environment.Stdout"#,
    );
    assert_eq!(run_image(&f.image(), "a\r\nb\n\nc", Options::default()).unwrap(), "a\nb\n\nc\n");
}
