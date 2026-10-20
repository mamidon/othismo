//! Acceptance tests over examples/, which need the AssemblyScript modules
//! built first: `make wasm` (or just `make test`).

mod common;

use common::run_image;
use real_talk::runtime::Options;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn example(rel: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(rel);
    let scripts = path.parent().unwrap().join("scripts");
    for ts in std::fs::read_dir(&scripts).unwrap().flatten().map(|e| e.path()) {
        if ts.extension().is_some_and(|e| e == "ts") {
            assert!(ts.with_extension("wasm").exists(), "{} is not compiled; run `make wasm`", ts.display());
        }
    }
    path
}

#[test]
fn hello_world_echoes_lines_through_the_binary() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_real_talk"))
        .arg(example("hello/hello.img"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all("hello\nhéllo 🦀\n".as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "hello\nhéllo 🦀\n");
}

#[test]
fn hello_world_with_shape_checks() {
    let out = run_image(&example("hello/hello.img"), "a\nb\n", Options { check_shapes: true }).unwrap();
    assert_eq!(out, "a\nb\n");
}

#[test]
fn greeter_uses_a_startup_request_and_a_second_object() {
    let out = run_image(&example("greeter/greeter.img"), "world\nAda\n", Options { check_shapes: true }).unwrap();
    assert_eq!(
        out,
        "Hello, world!\n\
         Stats {\"lines\": 1, \"chars\": 5, \"average\": 5.0}\n\
         HELLO, WORLD!\n\
         Hello, Ada!\n\
         Stats {\"lines\": 2, \"chars\": 8, \"average\": 4.0}\n\
         HELLO, ADA!\n"
    );
}
