//! Building, sealing and opening images.

mod common;

use common::{Fixture, module, module_with};
use othismo::pack::{self, BuildOptions, Pack};
use othismo::slots::{self, Given, GivenValue};
use othismo::tree::Node;

const A: &str = "export object A { needs /dev/cons { / write */* }  serves { / write */* } }";

/// Modules aren't run here, so they needn't do anything; they only need headers.
fn a() -> Vec<u8> {
    module("(module)", A)
}

fn fixture(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.file("scripts/A.wasm", a())
        .file("scripts/A.oth", A)
        .file("scripts/A.ts", "// A's source\n")
        .file("scripts/B.wasm", module("(module)", "object B {}"))
        .file(
            "test.oth",
            r#"import { A } from "./scripts/A.oth"
            import { B } from "./scripts/B.wasm"
            import { Console } from env
            needs /dev/cons Console
            instance a : A { bind /dev/cons /dev/cons }  mount /a a  write /a "ping"
            instance b : B { mount /a a readonly }
            instance a2 : A { bind /dev/cons /dev/cons }"#,
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

    // a and a2 share A's module.
    assert_eq!(built.modules.len(), 2);
    assert_eq!(built.blob(&built.modules["A"]), a());
    let sources: Vec<&str> = built.sources.keys().map(String::as_str).collect();
    assert_eq!(
        sources,
        ["scripts/A.oth", "scripts/A.ts", "test.oth"],
        "B has no header file or TypeScript, which is fine: its module carries its header"
    );
}

#[test]
fn sources_are_optional() {
    let f = fixture("pack_no_sources");
    let built = pack::build(&f.image(), BuildOptions { no_sources: true }).unwrap();
    assert!(built.sources.is_empty());
    assert_eq!(built.blobs.len(), 2, "just the modules");
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

    let mut old = sealed.clone();
    old[7] = 1;
    let e = err(&old);
    let want = format!("built by another version of othismo (format 1, and this one reads {}); rebuild it", pack::VERSION);
    assert!(e.contains(&want), "{e}");
    assert!(pack::is_sealed(&old), "still recognised as a built image, so it isn't parsed as a .oth");
}

#[test]
fn load_tells_built_images_from_sources() {
    let f = fixture("pack_load");
    let rti = f.dir.join("test.rti");
    std::fs::write(&rti, pack::build(&f.image(), BuildOptions::default()).unwrap().seal()).unwrap();
    let loaded = pack::load(&rti).unwrap();
    assert_eq!(loaded.dir, f.dir);
    assert!(loaded.program.sources.is_empty());
    assert_eq!(pack::load(&f.image()).unwrap().program.sources.len(), 2, "the image and A's header");

    let e = format!("{:#}", pack::build(&rti, BuildOptions::default()).unwrap_err());
    assert!(e.contains("is already built"), "{e}");
}

#[test]
fn unpacking_writes_modules_and_sources() {
    let f = fixture("pack_unpack");
    let out = f.dir.join("out");
    pack::build(&f.image(), BuildOptions::default()).unwrap().unpack(&out).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(rel)).unwrap();
    assert_eq!(std::fs::read(out.join("modules/A.wasm")).unwrap(), a());
    assert!(out.join("modules/B.wasm").exists());
    assert_eq!(read("sources/scripts/A.ts"), "// A's source\n");
    assert!(read("sources/test.oth").contains("instance a2 : A"));
}

/// An image whose only instance is `files : PackedFiles { <includes> }`.
fn packed(f: &Fixture, includes: &str) -> anyhow::Result<Pack> {
    f.file("test.oth", format!("import {{ PackedFiles }} from env instance files : PackedFiles {{ {includes} }}"));
    pack::build(&f.image(), BuildOptions::default())
}

fn listing(p: &Pack) -> Vec<String> {
    p.trees["files"]
        .nodes()
        .map(|(path, node)| match node {
            Node::Dir => format!("{}/", path.trim_end_matches('/')),
            Node::File { hash, .. } => format!("{path} = {}", String::from_utf8_lossy(p.blob(hash))),
        })
        .collect()
}

fn dir(f: &Fixture, rel: &str) {
    std::fs::create_dir_all(f.dir.join(rel)).unwrap();
}

#[test]
fn includes_take_files_and_whole_directories() {
    let f = Fixture::new("pack_includes");
    dir(&f, "public/css");
    dir(&f, "public/.well-known");
    dir(&f, "public/empty");
    f.file("public/index.html", "home").file("public/css/site.css", "css").file("public/.well-known/x", "hidden");
    f.file("robots.txt", "bots").file(".env", "dot");

    let p = packed(&f, r#"include "public/" include "robots.txt" include ".env" at /conf/env"#).unwrap();
    assert_eq!(
        listing(&p),
        [
            "/",
            "/.well-known/",
            "/.well-known/x = hidden",
            "/conf/",
            "/conf/env = dot",
            "/css/",
            "/css/site.css = css",
            "/empty/",
            "/index.html = home",
            "/robots.txt = bots",
        ]
    );
    assert_eq!(listing(&packed(&f, r#"include "public/*""#).unwrap()), listing(&packed(&f, r#"include "public/""#).unwrap()));
    let under = packed(&f, r#"include "public/css/" at /static"#).unwrap();
    assert_eq!(listing(&under), ["/", "/static/", "/static/site.css = css"]);
}

#[test]
fn later_includes_win() {
    let f = Fixture::new("pack_later_wins");
    dir(&f, "public/css");
    dir(&f, "theme/css");
    dir(&f, "theme/index.html");
    f.file("public/css/site.css", "plain").file("public/css/keep.css", "kept").file("public/index.html", "home");
    f.file("theme/css/site.css", "themed").file("theme/index.html/x", "now a dir");
    f.file("css.txt", "a file over a directory");

    let p = packed(&f, r#"include "public/" include "theme/""#).unwrap();
    assert_eq!(
        listing(&p),
        ["/", "/css/", "/css/keep.css = kept", "/css/site.css = themed", "/index.html/", "/index.html/x = now a dir"]
    );
    assert_eq!(p.log, ["files: include \"theme/\" replaced /css/site.css", "files: include \"theme/\" replaced /index.html"]);

    let p = packed(&f, r#"include "public/" include "css.txt" at /css"#).unwrap();
    assert_eq!(listing(&p), ["/", "/css = a file over a directory", "/index.html = home"]);
}

#[cfg(unix)]
#[test]
fn symlinks_are_followed() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("pack_symlinks");
    dir(&f, "public");
    dir(&f, "elsewhere/deep");
    f.file("elsewhere/deep/a.txt", "via a link").file("secret.txt", "outside");
    symlink(f.dir.join("elsewhere"), f.dir.join("public/linked")).unwrap();
    symlink(f.dir.join("secret.txt"), f.dir.join("public/secret.txt")).unwrap();
    let p = packed(&f, r#"include "public/""#).unwrap();
    assert_eq!(
        listing(&p),
        ["/", "/linked/", "/linked/deep/", "/linked/deep/a.txt = via a link", "/secret.txt = outside"]
    );

    symlink(f.dir.join("nowhere"), f.dir.join("public/dangling")).unwrap();
    let e = format!("{:#}", packed(&f, r#"include "public/""#).unwrap_err());
    assert!(e.contains("include \"public/\"") && e.contains("public/dangling is a symlink to nothing"), "{e}");
    std::fs::remove_file(f.dir.join("public/dangling")).unwrap();

    symlink(f.dir.join("public"), f.dir.join("elsewhere/deep/up")).unwrap();
    let e = format!("{:#}", packed(&f, r#"include "public/""#).unwrap_err());
    assert!(e.contains("links back to"), "{e}");
}

#[test]
fn include_mistakes_are_build_errors() {
    let f = Fixture::new("pack_include_errors");
    dir(&f, "public");
    f.file("a.txt", "a");
    let err = |includes: &str| format!("{:#}", packed(&f, includes).unwrap_err());
    let e = err(r#"include "public""#);
    assert!(e.contains("is a directory; write \"public/\" to include everything under it"), "{e}");
    assert!(err(r#"include "a.txt/""#).contains("a.txt is not a directory"));
    assert!(err(r#"include "missing.txt""#).contains("cannot include"));
    assert!(err(r#"include "a.txt" at /"#).contains("a single file can't be the root"));
    assert!(packed(&f, r#"include "public/""#).is_ok(), "an empty directory is fine");
}

#[test]
fn packed_trees_survive_sealing_and_unpacking() {
    let f = Fixture::new("pack_trees_sealed");
    dir(&f, "public/empty");
    f.file("public/a.txt", "A");
    let built = packed(&f, r#"include "public/""#).unwrap();
    let opened = pack::unseal(&built.seal(), f.dir.clone()).unwrap();
    assert_eq!(listing(&opened), listing(&built));
    assert_eq!(pack::describe(&opened, &othismo::formats()), "files  serves 1 built-in file, 1 bytes\n");

    let out = f.dir.join("out");
    opened.unpack(&out).unwrap();
    assert_eq!(std::fs::read_to_string(out.join("files/files/a.txt")).unwrap(), "A");
    assert!(out.join("files/files/empty").is_dir());
}

#[test]
fn slots_are_sealed_unbound_and_described_once_bound() {
    let f = Fixture::new("pack_slots");
    f.file(
        "test.oth",
        r#"import { Console, Files, Listener } from env
        require listen : text/plain = "127.0.0.1:8080"
        require root : text/plain
        needs /net   Listener
        needs /files Files
        needs /cons  Console
        write /net listen
        write /files root"#,
    );
    let sealed = pack::build(&f.image(), BuildOptions::default()).unwrap().seal();
    let mut opened = pack::unseal(&sealed, f.dir.clone()).unwrap();
    let e = format!("{:#}", slots::bind(&mut opened.program.clone(), &othismo::formats(), &[]).unwrap_err());
    assert!(e.ends_with("root: required, not provided"), "{e}");

    let given = Given { origin: "--set".into(), slot: "root".into(), field: None, value: GivenValue::Text("www".into()) };
    slots::bind(&mut opened.program, &othismo::formats(), &[given]).unwrap();
    assert_eq!(
        pack::describe(&opened, &othismo::formats()),
        "slot listen : text/plain = \"127.0.0.1:8080\" (default)\n\
         slot root : text/plain = \"www\" (--set)\n\
         \n\
         from the host:\n\
         \x20 /net    listens on 127.0.0.1:8080\n\
         \x20 /files  reads host files under www\n\
         \x20 /cons   reads stdin, writes stdout\n\
         \n"
    );
}

#[test]
fn the_cli_layers_value_files_then_set() {
    let f = Fixture::new("pack_cli_values");
    f.file(
        "test.oth",
        r#"import { A } from "./scripts/A.wasm"
        import { Listener } from env
        require listen : text/plain = "127.0.0.1:8080"
        require info : application/bson = { $type: "Info" }
        instance a : A
        needs /net Listener
        write /net listen"#,
    )
    .file("scripts/A.wasm", module_with("(module)", "object A {}", "message Info { kind: string, size: i64, mime: string }"))
    .file("a.vals", "listen = \"a\"\ninfo = { kind: \"file\", size: 1, mime: \"\" }")
    .file("b.vals", "info = { size: 2 }");
    let check = |extra: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_othismo"))
            .current_dir(&f.dir)
            .arg("--check")
            .args(extra)
            .arg(f.image())
            .output()
            .unwrap();
        (out.status.code(), String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap())
    };
    let (code, out) = check(&["--set", "listen=0.0.0.0:0", "--with", "a.vals", "--with", "b.vals"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.starts_with("slot listen : text/plain = \"0.0.0.0:0\" (--set)\n"), "{out}");
    assert!(
        out.contains("slot info : application/bson { $type: \"Info\" (default), kind: \"file\" (a.vals:2:10), size: 2 (b.vals:1:10), mime: \"\" (a.vals:2:33) }"),
        "{out}"
    );

    let (code, out) = check(&[]);
    assert_eq!(code, Some(1));
    assert!(out.contains("info.kind: required, not provided"), "{out}");
    let (code, out) = check(&["--set", "nonsense"]);
    assert_eq!(code, Some(1));
    assert!(out.contains("expected slot=value or slot.field=value"), "{out}");
    assert_eq!(check(&["--set"]).0, Some(2), "a missing argument is a usage error");
}
