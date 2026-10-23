//! Binding slots: defaults, value files and `--set`, in a checked image.

use format::Format;
use format_bson::Bson;
use othismo::check::{self, Arg, Program};
use othismo::image_definition::Value;
use othismo::payload::Payload;
use othismo::slots::{Given, GivenValue, bind, parse_set, read_values};
use othismo::wire;
use std::path::Path;

/// Checks `src`, whose one module, `A`, takes what IMAGE writes to it
/// and declares `Info`.
fn program(src: &str) -> Program {
    let a = "export object A { serves { /info write application/bson  /x write */*  /host write text/plain } }";
    let info = "message Info { kind: string, size: i64, mime: string }";
    let header = othismo::iface::resolve(&othismo::iface::parse(a).unwrap(), &mut |_| Ok(othismo::iface::env().clone()), &|_| {
        unreachable!()
    })
    .unwrap();
    let wasm = othismo::iface::embed(&wat::parse_str("(module)").unwrap(), &header).unwrap();
    let wasm = othismo::iface::embed_schemas(&wasm, &Bson, &Bson.parse(info, &mut |_| unreachable!()).unwrap()).unwrap();
    let mut loader = othismo::iface::Loader::new(othismo::formats());
    loader.files.insert("/slots/A.oth".into(), format!("{a}\nexport schemas application/bson {{ {info} }}").into_bytes());
    loader.files.insert("/slots/A.wasm".into(), wasm);
    let src = format!("import {{ A }} from \"./A.oth\" import {{ Console, Listener }} from env {src}");
    loader.files.insert("/slots/test.oth".into(), src.into_bytes());
    check::load_with(Path::new("/slots/test.oth"), &mut loader).unwrap()
}

const IMAGE: &str = r#"
    require listen : text/plain = "127.0.0.1:8080"
    require info : application/bson = { $type: "Info", kind: "file" }
    require host : text/plain = "localhost"
    needs /net Listener
    instance a : A
    mount /a a
    write /net listen
    write /a/info info
    write /a/x "literal"
    write /a/host host
"#;

fn given(origin: &str, slot: &str, field: Option<&str>, value: GivenValue) -> Given {
    Given { origin: origin.into(), slot: slot.into(), field: field.map(Into::into), value }
}

fn text(origin: &str, slot: &str, field: &str, value: &str) -> Given {
    given(origin, slot, Some(field), GivenValue::Text(value.into()))
}

fn whole(origin: &str, slot: &str, value: &str) -> Given {
    given(origin, slot, None, GivenValue::Text(value.into()))
}

fn fields(arg: &Arg) -> bson::Document {
    wire::Message::decode(&arg.payload().unwrap().bytes).unwrap().fields
}

#[test]
fn later_values_win_field_by_field() {
    let mut p = program(IMAGE);
    let values = [
        given("a.vals:2:3", "listen", None, GivenValue::Literal(Value::Str("0.0.0.0:80".into()))),
        text("--set", "info", "size", "1"),
        text("--set", "info", "mime", "text/plain"),
        whole("--set", "listen", "0.0.0.0:8080"),
        text("--set", "info", "size", "42"),
    ];
    bind(&mut p, &othismo::formats(), &values).unwrap();

    assert_eq!(p.instances[0].config.as_ref().unwrap().text().unwrap(), "0.0.0.0:8080");
    let info = fields(p.startup[0].body.as_ref().unwrap());
    assert_eq!(info, bson::doc! { "kind": "file", "size": 42i64, "mime": "text/plain" }, "in declared order");
    let origins = |i: usize| p.slots[i].origins.iter().map(|(f, o)| format!("{f}={o}")).collect::<Vec<_>>();
    assert_eq!(origins(0), ["=--set"]);
    assert_eq!(origins(1), ["$type=default", "kind=default", "size=--set", "mime=--set"]);
    assert_eq!(p.startup[2].body.as_ref().unwrap().text().unwrap(), "localhost", "a default");
}

#[test]
fn payload_slots_are_given_whole() {
    let mut p = program(IMAGE);
    let info = [text("--set", "info", "size", "1"), text("--set", "info", "mime", "")];
    bind(&mut p, &othismo::formats(), &[&info[..], &[whole("a.vals:1:8", "host", "a"), whole("--set", "host", "example.com")]].concat()).unwrap();
    assert_eq!(p.startup[2].body, Some(Arg::Payload(Payload::text("example.com"))));
    assert_eq!(p.slots[2].origins, [(String::new(), "--set".to_string())]);

    let mut p = program("require host : text/plain instance a : A mount /a a write /a/host host");
    let e = bind(&mut p, &othismo::formats(), &[]).unwrap_err().to_string();
    assert!(e.ends_with("host: required, not provided"), "{e}");
}

#[test]
fn a_whole_structured_value_merges_with_its_default() {
    let mut p = program(IMAGE);
    let tree = |src: &str| {
        let mut parser = othismo::image_definition::Parser::new(src, false).unwrap();
        parser.value().unwrap()
    };
    bind(&mut p, &othismo::formats(), &[given("v.vals:1:8", "info", None, GivenValue::Literal(tree(r#"{ size: 3, mime: "" }"#)))]).unwrap();
    assert_eq!(fields(p.startup[0].body.as_ref().unwrap()), bson::doc! { "kind": "file", "size": 3i64, "mime": "" });
}

#[test]
fn every_problem_is_listed() {
    let mut p = program(IMAGE);
    let values = [
        text("--set", "lisen", "address", "x"),
        text("--set", "info", "port", "80"),
        text("--set", "info", "size", "big"),
        text("--set", "host", "name", "x"),
        whole("--set", "info", "x"),
        given("v.vals:2:8", "host", None, GivenValue::Literal(Value::Int(1))),
    ];
    let e = bind(&mut p, &othismo::formats(), &values).unwrap_err().to_string();
    assert_eq!(
        e,
        "the image's slots can't be filled:\n  \
         --set: there is no slot `lisen`\n  \
         --set: info.port: `Info` has no field `port`\n  \
         --set: info.size: `big` is not a valid i64\n  \
         --set: host.name: `host` is a text/plain payload, given whole: `host=...`\n  \
         --set: info: `info` is a application/bson payload, given field by field: `info.<field>=...`\n  \
         v.vals:2:8: host: expected a quoted string, got 1\n  \
         info.size: required, not provided\n  \
         info.mime: required, not provided"
    );
    let mut p = program(r#"require g : application/bson instance a : A mount /a a write /a/info g"#);
    let e = bind(&mut p, &othismo::formats(), &[text("--set", "g", "kind", "x")]).unwrap_err().to_string();
    assert!(e.contains("g.kind: give `$type` first"), "{e}");
    assert!(e.contains("g.$type: required, not provided"), "{e}");
}

#[test]
fn values_come_from_files_and_set() {
    let dir = std::env::temp_dir().join(format!("othismo_values_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("prod.vals");
    std::fs::write(&file, "listen = \"0.0.0.0:80\"\ninfo = { size: 1 }").unwrap();
    let values = read_values(&file).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0], given(&format!("{}:1:10", file.display()), "listen", None, GivenValue::Literal(Value::Str("0.0.0.0:80".into()))));
    assert_eq!((values[1].slot.as_str(), values[1].field.as_deref()), ("info", Some("size")));
    assert_eq!(values[1].origin, format!("{}:2:10", file.display()));
    assert_eq!(values[1].value, GivenValue::Literal(Value::Int(1)));

    assert_eq!(parse_set("listen.address=0.0.0.0:80=x").unwrap(), text("--set", "listen", "address", "0.0.0.0:80=x"));
    assert_eq!(parse_set("a.b=").unwrap(), text("--set", "a", "b", ""));
    assert_eq!(parse_set("listen=1").unwrap(), whole("--set", "listen", "1"));
    for bad in ["listen.address", ".address=1", "listen.=1", "=1"] {
        assert!(parse_set(bad).unwrap_err().to_string().contains("expected slot=value or slot.field=value"), "{bad}");
    }
}

#[test]
fn an_image_without_slots_needs_nothing() {
    let mut p = program("needs /cons Console");
    bind(&mut p, &othismo::formats(), &[]).unwrap();
    let e = bind(&mut p, &othismo::formats(), &[text("--set", "x", "y", "z")]).unwrap_err().to_string();
    assert!(e.contains("there is no slot `x`"), "{e}");
}
