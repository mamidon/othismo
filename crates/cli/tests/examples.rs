//! Acceptance tests over examples/, which need the AssemblyScript modules
//! built first: `task test`.

mod common;

use common::run_image;
use othismo::runtime::Options;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn example(rel: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(rel);
    let scripts = path.parent().unwrap().join("scripts");
    for ts in std::fs::read_dir(&scripts).unwrap().flatten().map(|e| e.path()) {
        // A module's own source, not its generated bindings.
        let generated = [".messages.ts", ".oth.ts"].iter().any(|g| ts.to_string_lossy().ends_with(g));
        if ts.extension().is_some_and(|e| e == "ts") && !generated {
            assert!(ts.with_extension("wasm").exists(), "{} is not compiled; run `task test`", ts.display());
        }
    }
    path
}

/// Every example header's and schema file's bindings are generated and
/// committed; this fails if one is stale, or missing. `task bindings`
/// regenerates them.
#[test]
fn bindings_are_up_to_date() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut seen = 0;
    for dir in std::fs::read_dir(&examples).unwrap().flatten() {
        let Ok(scripts) = std::fs::read_dir(dir.path().join("scripts")) else { continue };
        for source in scripts.flatten().map(|e| e.path()) {
            let name = source.file_name().unwrap().to_string_lossy().into_owned();
            let want = match source.extension().and_then(|e| e.to_str()) {
                Some("messages") => {
                    use format::Format;
                    let bson = &format_bson::Bson;
                    let schemas = othismo::iface::Loader::new(othismo::formats()).schemas(bson, &source).unwrap();
                    bson.bindings(&schemas, &name, "othismo/bson")
                }
                Some("oth") => othismo::bindings::namespace(&othismo::iface::load(&source).unwrap(), &name, "othismo", "othismo/env"),
                _ => continue,
            };
            seen += 1;
            let committed = std::fs::read_to_string(format!("{}.ts", source.display())).ok();
            assert!(committed == want, "{}.ts is stale; run `task bindings`", source.display());
        }
    }
    assert!(seen > 0);
}

#[test]
fn hello_world_echoes_lines_through_the_binary() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_othismo"))
        .arg(example("hello/hello.oth"))
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
fn hello_world_with_checks_off() {
    let out = run_image(&example("hello/hello.oth"), "a\nb\n", Options { no_checks: true }).unwrap();
    assert_eq!(out, "a\nb\n");
}

#[test]
fn greeter_uses_a_startup_request_and_a_second_object() {
    let out = run_image(&example("greeter/greeter.oth"), "world\nAda\n", Options::default()).unwrap();
    assert_eq!(
        out,
        "Hello, world!\n\
         1 lines, 5 characters, 5.0 on average\n\
         HELLO, WORLD!\n\
         Hello, Ada!\n\
         2 lines, 8 characters, 4.0 on average\n\
         HELLO, ADA!\n"
    );
}

#[test]
fn a_built_image_runs_without_its_sources() {
    let dir = std::env::temp_dir().join(format!("othismo_built_hello_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rti = dir.join("hello.rti");
    let bin = env!("CARGO_BIN_EXE_othismo");
    let built = Command::new(bin).arg("build").arg(example("hello/hello.oth")).arg("-o").arg(&rti).output().unwrap();
    assert!(built.status.success(), "stderr: {}", String::from_utf8_lossy(&built.stderr));

    // Nothing but the .rti is next to it, and it runs the same.
    let out = run_image(&rti, "a\nb\n", Options::default()).unwrap();
    assert_eq!(out, "a\nb\n");

    let unpacked = Command::new(bin).arg("unpack").arg(&rti).arg(dir.join("out")).output().unwrap();
    assert!(unpacked.status.success(), "stderr: {}", String::from_utf8_lossy(&unpacked.stderr));
    assert!(dir.join("out/modules/Echo.wasm").exists());
    assert!(dir.join("out/sources/scripts/Echo.ts").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every example's test suites pass: each `<name>.test.oth` beside an
/// example's image, run as `othismo test` runs it.
#[test]
fn example_tests_pass() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut seen = 0;
    for dir in std::fs::read_dir(&examples).unwrap().flatten() {
        for img in std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.path()) {
            if !img.to_string_lossy().ends_with(".test.oth") {
                continue;
            }
            let rel = img.strip_prefix(&examples).unwrap().to_string_lossy().into_owned();
            let mut pack = othismo::pack::load(&example(&rel)).unwrap();
            othismo::slots::bind(&mut pack.program, &othismo::formats(), &[]).unwrap();
            let reports = othismo::suite::run(&pack, Options::default()).unwrap();
            assert!(!reports.is_empty(), "{rel} has no tests");
            let failed: Vec<_> = reports.iter().filter(|r| !r.passed).collect();
            assert!(failed.is_empty(), "{rel}: {failed:#?}");
            seen += reports.len();
        }
    }
    assert!(seen >= 6, "the static site's tests ran");
}
