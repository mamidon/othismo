//! Building, sealing and opening images.

mod common;

use common::Fixture;
use real_talk::pack::{self, BuildOptions};

/// Modules aren't run here, so any bytes do.
fn fixture(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.file("messages.ts", "export class Ping {}\n")
        .file("scripts/A.wasm", "(module)")
        .file("scripts/A.ts", "// A's source\n")
        .file("scripts/B.wasm", "(module)")
        .file(
            "test.img",
            r#"import { Ping } from "./messages"
            instance cons : Environment.Console
            instance a : A { mount cons /dev/cons  invoke / Ping }
            instance b : B { mount a /a readonly }
            instance a2 : A"#,
        );
    f
}

#[test]
fn sealing_round_trips_and_is_reproducible() {
    let f = fixture("pack_round_trip");
    let built = pack::build(&f.image(), BuildOptions::default()).unwrap();
    let sealed = built.seal();
    assert_eq!(pack::build(&f.image(), BuildOptions::default()).unwrap().seal(), sealed, "same inputs, same bytes");

    let opened = pack::unseal(&sealed, f.dir.clone()).unwrap();
    assert_eq!(opened.program.instances, built.program.instances);
    assert_eq!(opened.program.messages, built.program.messages);
    assert!(opened.program.sources.is_empty(), "where it was built from isn't kept");
    assert_eq!(opened.modules, built.modules);
    assert_eq!(opened.blobs, built.blobs);

    // A and B have identical modules, stored once.
    assert_eq!(built.modules.len(), 2);
    assert_eq!(built.modules["A"], built.modules["B"]);
    assert_eq!(built.blob(&built.modules["A"]), b"(module)");
    let sources: Vec<&str> = built.sources.keys().map(String::as_str).collect();
    assert_eq!(sources, ["messages.ts", "scripts/A.ts", "test.img"], "B has no TypeScript, which is fine");
}

#[test]
fn sources_are_optional() {
    let f = fixture("pack_no_sources");
    let built = pack::build(&f.image(), BuildOptions { no_sources: true }).unwrap();
    assert!(built.sources.is_empty());
    assert_eq!(built.blobs.len(), 1, "just the module");
}

#[test]
fn corrupt_and_truncated_files_are_refused() {
    let f = fixture("pack_corrupt");
    let sealed = pack::build(&f.image(), BuildOptions::default()).unwrap().seal();
    let err = |bytes: &[u8]| format!("{:#}", pack::unseal(bytes, f.dir.clone()).unwrap_err());

    let mut flipped = sealed.clone();
    *flipped.last_mut().unwrap() ^= 1;
    assert!(err(&flipped).contains("is corrupt"), "{}", err(&flipped));
    assert!(err(&sealed[..sealed.len() - 1]).contains("out of bounds"));
    assert!(err(&sealed[..20]).contains("truncated manifest"));
    assert!(err(b"instance a : A").contains("not a built image"));
}

#[test]
fn load_tells_built_images_from_sources() {
    let f = fixture("pack_load");
    let rti = f.dir.join("test.rti");
    std::fs::write(&rti, pack::build(&f.image(), BuildOptions::default()).unwrap().seal()).unwrap();
    let loaded = pack::load(&rti).unwrap();
    assert_eq!(loaded.dir, f.dir);
    assert!(loaded.program.sources.is_empty());
    assert_eq!(pack::load(&f.image()).unwrap().program.sources.len(), 2);

    let e = format!("{:#}", pack::build(&rti, BuildOptions::default()).unwrap_err());
    assert!(e.contains("is already built"), "{e}");
}

#[test]
fn unpacking_writes_modules_and_sources() {
    let f = fixture("pack_unpack");
    let out = f.dir.join("out");
    pack::build(&f.image(), BuildOptions::default()).unwrap().unpack(&out).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(rel)).unwrap();
    assert_eq!(read("modules/A.wasm"), "(module)");
    assert_eq!(read("modules/B.wasm"), "(module)");
    assert_eq!(read("sources/scripts/A.ts"), "// A's source\n");
    assert!(read("sources/test.img").contains("instance a2 : A"));
}
