//! Binding slots: filling each `require`d value from the image's defaults
//! and the values the host gives, then putting the results wherever the
//! slots are used. See `docs/images.md`.

use crate::check::{self, Arg, Program, SlotKind};
use crate::image::{SlotValue, Value};
use crate::messages::FieldType;
use crate::payload::Payload;
use crate::wire::Message;
use anyhow::{Context, Result, bail};
use bson::Document;
use std::path::Path;

/// One value from the host: a whole slot's, or one field of a message's.
#[derive(Debug, Clone, PartialEq)]
pub struct Given {
    /// Where it came from, for errors and `--check`: `prod.vals:3:5`, `--set`.
    pub origin: String,
    pub slot: String,
    /// The field, for a message; `None` for a payload, given whole.
    pub field: Option<String>,
    pub value: GivenValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GivenValue {
    /// A literal, from a value file.
    Literal(Value),
    /// Raw text, from `--set`: kept as is for a payload or a `string` or
    /// `bytes` field, and read as a literal for any other.
    Text(String),
}

/// The values in a value file, in order.
pub fn read_values(path: &Path) -> Result<Vec<Given>> {
    let src = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let blocks = crate::image::parse_values(&src).with_context(|| format!("parsing {}", path.display()))?;
    let mut out = vec![];
    for block in blocks {
        let slot = block.slot.name.clone();
        match block.value {
            SlotValue::Text(text, pos) => out.push(Given {
                origin: format!("{}:{pos}", path.display()),
                slot,
                field: None,
                value: GivenValue::Text(text),
            }),
            SlotValue::Fields(fields) => {
                for (field, value) in fields {
                    out.push(Given {
                        origin: format!("{}:{}", path.display(), field.pos),
                        slot: slot.clone(),
                        field: Some(field.name),
                        value: GivenValue::Literal(value),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// The value in a `--set slot=value` or `--set slot.field=value` argument.
pub fn parse_set(arg: &str) -> Result<Given> {
    let Some((key, value)) = arg.split_once('=') else { bail!("--set {arg}: expected slot=value or slot.field=value") };
    let (slot, field) = match key.split_once('.') {
        Some((slot, field)) => (slot, Some(field)),
        None => (key, None),
    };
    if slot.is_empty() || field.is_some_and(str::is_empty) {
        bail!("--set {arg}: expected slot=value or slot.field=value");
    }
    Ok(Given { origin: "--set".into(), slot: slot.into(), field: field.map(Into::into), value: GivenValue::Text(value.into()) })
}

/// What a slot has so far: a payload's text, or a message's fields, with
/// where each came from.
enum Filling {
    Payload { ty: String, text: Option<(String, String)> },
    Message { fields: Document, origins: Vec<(String, String)> },
}

/// Fills every slot: the image's defaults first, then `given` in order, a
/// later value replacing an earlier one. Fails, listing every problem, if a
/// value names an unknown slot or field or doesn't fit, or if any slot or
/// field is left without a value.
pub fn bind(program: &mut Program, given: &[Given]) -> Result<()> {
    let mut problems = vec![];
    let mut filling: Vec<Filling> = program
        .slots
        .iter()
        .map(|s| match &s.kind {
            SlotKind::Payload { ty, default } => {
                Filling::Payload { ty: ty.clone(), text: default.clone().map(|d| (d, "default".to_string())) }
            }
            SlotKind::Message { defaults, .. } => Filling::Message {
                fields: defaults.clone(),
                origins: defaults.keys().map(|k| (k.clone(), "default".to_string())).collect(),
            },
        })
        .collect();

    for g in given {
        let Some(i) = program.slots.iter().position(|s| s.name == g.slot) else {
            problems.push(format!("{}: there is no slot `{}`", g.origin, g.slot));
            continue;
        };
        match (&mut filling[i], &program.slots[i].kind, &g.field) {
            (Filling::Payload { text, .. }, _, None) => match &g.value {
                GivenValue::Text(t) | GivenValue::Literal(Value::Str(t)) => *text = Some((t.clone(), g.origin.clone())),
                GivenValue::Literal(v) => problems.push(format!("{}: {}: expected a quoted string, got {v:?}", g.origin, g.slot)),
            },
            (Filling::Payload { ty, .. }, _, Some(f)) => {
                problems.push(format!("{}: {}.{f}: `{}` is a {ty} payload, given whole: `{}=...`", g.origin, g.slot, g.slot, g.slot))
            }
            (Filling::Message { .. }, SlotKind::Message { type_name, .. }, None) => {
                problems.push(format!("{}: {}: `{}` is a {type_name}, given field by field: `{}.<field>=...`", g.origin, g.slot, g.slot, g.slot))
            }
            (Filling::Message { fields, origins }, SlotKind::Message { type_name, .. }, Some(field)) => {
                let ty = &program.messages[type_name];
                let value = match &g.value {
                    GivenValue::Literal(v) => Ok(v.clone()),
                    GivenValue::Text(text) => match ty.field(field) {
                        Some(t) => literal(t, text),
                        None => Err(format!("`{}` has no field `{field}`", ty.name)),
                    },
                };
                match value.and_then(|v| check::field_bson(ty, field, &v)) {
                    Ok(bson) => {
                        fields.insert(field.clone(), bson);
                        origins.retain(|(f, _)| f != field);
                        origins.push((field.clone(), g.origin.clone()));
                    }
                    Err(e) => problems.push(format!("{}: {}.{field}: {e}", g.origin, g.slot)),
                }
            }
            (Filling::Message { .. }, SlotKind::Payload { .. }, _) => unreachable!("fillings follow their slots"),
        }
    }

    let mut bound = vec![];
    for (slot, filled) in program.slots.iter_mut().zip(filling) {
        let (value, origins) = match (filled, &slot.kind) {
            (Filling::Payload { ty, text: Some((text, origin)) }, _) => {
                (Arg::Payload(Payload::new(ty, text.as_bytes())), vec![(String::new(), origin)])
            }
            (Filling::Payload { .. }, _) => {
                problems.push(format!("{}: required, not provided", slot.name));
                (Arg::Payload(Payload::text("")), vec![])
            }
            (Filling::Message { fields, origins }, SlotKind::Message { type_name, .. }) => {
                let ty = &program.messages[type_name];
                let mut ordered = Document::new();
                let mut in_order = vec![];
                for (name, _) in &ty.fields {
                    match fields.get(name) {
                        Some(v) => {
                            ordered.insert(name.clone(), v.clone());
                            in_order.extend(origins.iter().find(|(f, _)| f == name).cloned());
                        }
                        None => problems.push(format!("{}.{name}: required, not provided", slot.name)),
                    }
                }
                (Arg::Message(Message { type_name: type_name.clone(), fields: ordered }), in_order)
            }
            (Filling::Message { .. }, SlotKind::Payload { .. }) => unreachable!("fillings follow their slots"),
        };
        slot.value = Some(value.clone());
        slot.origins = origins;
        bound.push(value);
    }
    if !problems.is_empty() {
        bail!("the image's slots can't be filled:\n  {}", problems.join("\n  "));
    }

    let fill = |arg: &mut Arg| {
        if let Arg::Slot(i) = *arg {
            *arg = bound[i].clone();
        }
    };
    for inst in &mut program.instances {
        inst.config.iter_mut().for_each(fill);
    }
    program.startup.iter_mut().filter_map(|s| s.body.as_mut()).for_each(fill);
    Ok(())
}

/// Reads `text` as a literal for a field of type `t`.
fn literal(t: FieldType, text: &str) -> std::result::Result<Value, String> {
    let bad = || format!("`{text}` is not a valid {t}");
    Ok(match t {
        FieldType::String | FieldType::Bytes => Value::Str(text.to_string()),
        FieldType::Bool => match text {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(bad()),
        },
        FieldType::F32 | FieldType::F64 => Value::Float(text.parse().map_err(|_| bad())?),
        _ => Value::Int(text.parse().map_err(|_| bad())?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check;
    use std::path::Path;

    /// Checks `src`, whose one module, `A`, takes what IMAGE writes to it.
    fn program(src: &str) -> Program {
        let a = "export message Info { kind: string, size: i64, mime: string }
            export object A { serves { /info write application/bson  /x write */*  /host write text/plain } }";
        let header = crate::iface::resolve(&crate::iface::parse(a).unwrap(), &mut |_| Ok(crate::iface::env().clone()), &|_| {
            unreachable!()
        })
        .unwrap();
        let wasm = crate::iface::embed(&wat::parse_str("(module)").unwrap(), &header).unwrap();
        let mut loader = crate::iface::Loader::default();
        loader.files.insert("/slots/A.oth".into(), a.into());
        loader.files.insert("/slots/A.wasm".into(), wasm);
        let src = format!("import {{ A, Info }} from \"./A.oth\" import {{ Console, Listener }} from env {src}");
        loader.files.insert("/slots/test.oth".into(), src.into_bytes());
        check::load_with(Path::new("/slots/test.oth"), &mut loader).unwrap()
    }

    const IMAGE: &str = r#"
        require listen : text/plain = "127.0.0.1:8080"
        require info : Info { kind: "file" }
        require host : text/plain = "localhost"
        instance net : Listener
        instance a : A
        mount /net net
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
        bind(&mut p, &values).unwrap();

        assert_eq!(p.instances[0].config.as_ref().unwrap().text().unwrap(), "0.0.0.0:8080");
        let info = p.startup[0].body.as_ref().unwrap().message().unwrap();
        assert_eq!(info.fields, bson::doc! { "kind": "file", "size": 42i64, "mime": "text/plain" }, "in declared order");
        let origins = |i: usize| p.slots[i].origins.iter().map(|(f, o)| format!("{f}={o}")).collect::<Vec<_>>();
        assert_eq!(origins(0), ["=--set"]);
        assert_eq!(origins(1), ["kind=default", "size=--set", "mime=--set"]);
        assert_eq!(p.startup[2].body.as_ref().unwrap().text().unwrap(), "localhost", "a default");
    }

    #[test]
    fn payload_slots_are_given_whole() {
        let mut p = program(IMAGE);
        let info = [text("--set", "info", "size", "1"), text("--set", "info", "mime", "")];
        bind(&mut p, &[&info[..], &[whole("a.vals:1:8", "host", "a"), whole("--set", "host", "example.com")]].concat()).unwrap();
        assert_eq!(p.startup[2].body, Some(Arg::Payload(Payload::text("example.com"))));
        assert_eq!(p.slots[2].origins, [(String::new(), "--set".to_string())]);

        let mut p = program("require host : text/plain instance a : A mount /a a write /a/host host");
        let e = bind(&mut p, &[]).unwrap_err().to_string();
        assert!(e.ends_with("host: required, not provided"), "{e}");
    }

    #[test]
    fn every_problem_is_listed() {
        let mut p = program(IMAGE);
        let values = [
            text("--set", "lisen", "address", "x"),
            text("--set", "info", "port", "80"),
            text("--set", "info", "size", "big"),
            given("v.vals:1:8", "info", Some("kind"), GivenValue::Literal(Value::Int(1))),
            text("--set", "host", "name", "x"),
            whole("--set", "info", "x"),
            given("v.vals:2:8", "host", None, GivenValue::Literal(Value::Int(1))),
        ];
        let e = bind(&mut p, &values).unwrap_err().to_string();
        assert_eq!(
            e,
            "the image's slots can't be filled:\n  \
             --set: there is no slot `lisen`\n  \
             --set: info.port: `Info` has no field `port`\n  \
             --set: info.size: `big` is not a valid i64\n  \
             v.vals:1:8: info.kind: `Info.kind` is string, but was given Int(1)\n  \
             --set: host.name: `host` is a text/plain payload, given whole: `host=...`\n  \
             --set: info: `info` is a Info, given field by field: `info.<field>=...`\n  \
             v.vals:2:8: host: expected a quoted string, got Int(1)\n  \
             info.size: required, not provided\n  \
             info.mime: required, not provided"
        );
    }

    #[test]
    fn values_come_from_files_and_set() {
        let dir = std::env::temp_dir().join(format!("othismo_values_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prod.vals");
        std::fs::write(&file, "listen {\n  address: \"0.0.0.0:80\"\n}\ninfo { size: 1 }\nhost = \"x\"").unwrap();
        let values = read_values(&file).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0].origin, format!("{}:2:3", file.display()));
        assert_eq!((values[1].slot.as_str(), values[1].field.as_deref()), ("info", Some("size")));
        assert_eq!(values[1].value, GivenValue::Literal(Value::Int(1)));
        assert_eq!(values[2], given(&format!("{}:5:8", file.display()), "host", None, GivenValue::Text("x".into())));

        assert_eq!(parse_set("listen.address=0.0.0.0:80=x").unwrap(), text("--set", "listen", "address", "0.0.0.0:80=x"));
        assert_eq!(parse_set("a.b=").unwrap(), text("--set", "a", "b", ""));
        assert_eq!(parse_set("listen=1").unwrap(), whole("--set", "listen", "1"));
        for bad in ["listen.address", ".address=1", "listen.=1", "=1"] {
            assert!(parse_set(bad).unwrap_err().to_string().contains("expected slot=value or slot.field=value"), "{bad}");
        }
    }

    #[test]
    fn an_image_without_slots_needs_nothing() {
        let mut p = program("instance cons : Console");
        bind(&mut p, &[]).unwrap();
        let e = bind(&mut p, &[text("--set", "x", "y", "z")]).unwrap_err().to_string();
        assert!(e.contains("there is no slot `x`"), "{e}");
    }
}
