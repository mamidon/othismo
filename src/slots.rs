//! Binding slots: filling each of the root image's `require`d values from
//! its default and the values the host gives, then putting the results
//! wherever the slots are used. A structured slot is filled field by field
//! and then encoded by its format. See `docs/images.md`.

use crate::check::{Arg, Program, SlotValue};
use crate::format;
use crate::image::{Ident, Pos, Value};
use crate::messages::FieldType;
use crate::payload::Payload;
use crate::wire;
use anyhow::{Context, Result, bail};
use std::path::Path;

/// One value from the host: a whole slot's, or one field of a structured one's.
#[derive(Debug, Clone, PartialEq)]
pub struct Given {
    /// Where it came from, for errors and `--check`: `prod.vals:3:5`, `--set`.
    pub origin: String,
    pub slot: String,
    /// The field, for a structured slot given field by field.
    pub field: Option<String>,
    pub value: GivenValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GivenValue {
    /// A literal, from a value file.
    Literal(Value),
    /// Raw text, from `--set`: kept as is for a text slot or a `string` or
    /// `bytes` field, and read as a literal for any other.
    Text(String),
}

/// The values in a value file, in order: a structured value's field by field.
pub fn read_values(path: &Path) -> Result<Vec<Given>> {
    let src = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let blocks = crate::image::parse_values(&src).with_context(|| format!("parsing {}", path.display()))?;
    let origin = |pos: Pos| format!("{}:{pos}", path.display());
    let mut out = vec![];
    for block in blocks {
        let slot = block.slot.name.clone();
        match block.value {
            Value::Object(fields) => {
                for (field, value) in fields {
                    out.push(Given { origin: origin(field.pos), slot: slot.clone(), field: Some(field.name), value: GivenValue::Literal(value) });
                }
            }
            value => out.push(Given { origin: origin(block.pos), slot, field: None, value: GivenValue::Literal(value) }),
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

/// What a slot has so far: text, or a structured value's fields, each with
/// where it came from.
enum Filling {
    Text(Option<(String, String)>),
    Fields(Vec<(String, Value, String)>),
}

impl Filling {
    fn set(fields: &mut Vec<(String, Value, String)>, name: &str, value: Value, origin: &str) {
        fields.retain(|(f, ..)| f != name);
        fields.push((name.to_string(), value, origin.to_string()));
    }

    /// The type a structured value's `$type` names, so far.
    fn type_name(fields: &[(String, Value, String)]) -> Option<&str> {
        fields.iter().find_map(|(f, v, _)| match v {
            Value::Str(t) if f == wire::TYPE_KEY => Some(t.as_str()),
            _ => None,
        })
    }
}

/// Fills every slot: the image's defaults first, then `given` in order, a
/// later value replacing an earlier one, field by field for a structured
/// slot. Fails, listing every problem, if a value names an unknown slot or
/// field or doesn't fit, or if any slot or field is left without a value.
pub fn bind(program: &mut Program, given: &[Given]) -> Result<()> {
    let mut problems = vec![];
    let mut filling: Vec<Filling> = program
        .slots
        .iter()
        .map(|s| match (&s.default, s.structured()) {
            (Some(SlotValue::Tree(Value::Object(fields))), true) => {
                Filling::Fields(fields.iter().map(|(k, v)| (k.name.clone(), v.clone(), "default".to_string())).collect())
            }
            (_, true) => Filling::Fields(vec![]),
            (Some(SlotValue::Text(t)), false) => Filling::Text(Some((t.clone(), "default".to_string()))),
            (_, false) => Filling::Text(None),
        })
        .collect();

    for g in given {
        let Some(i) = program.slots.iter().position(|s| s.name == g.slot) else {
            problems.push(format!("{}: there is no slot `{}`", g.origin, g.slot));
            continue;
        };
        let slot = &program.slots[i];
        match (&mut filling[i], &g.field, &g.value) {
            (Filling::Text(text), None, GivenValue::Text(t) | GivenValue::Literal(Value::Str(t))) => {
                *text = Some((t.clone(), g.origin.clone()))
            }
            (Filling::Text(_), None, GivenValue::Literal(v)) => problems.push(format!("{}: {}: expected a quoted string, got {v}", g.origin, g.slot)),
            (Filling::Text(_), Some(f), _) => {
                problems.push(format!("{}: {}.{f}: `{}` is a {} payload, given whole: `{}=...`", g.origin, g.slot, g.slot, slot.ty, g.slot))
            }
            (Filling::Fields(fields), None, GivenValue::Literal(Value::Object(given))) => {
                for (k, v) in given {
                    Filling::set(fields, &k.name, v.clone(), &g.origin);
                }
            }
            (Filling::Fields(_), None, _) => problems.push(format!(
                "{}: {}: `{}` is a {} payload, given field by field: `{}.<field>=...`",
                g.origin, g.slot, g.slot, slot.ty, g.slot
            )),
            (Filling::Fields(fields), Some(f), GivenValue::Literal(v)) => Filling::set(fields, f, v.clone(), &g.origin),
            (Filling::Fields(fields), Some(f), GivenValue::Text(raw)) => {
                if f == wire::TYPE_KEY {
                    Filling::set(fields, f, Value::Str(raw.clone()), &g.origin);
                    continue;
                }
                let ty = Filling::type_name(fields).and_then(|t| program.messages.types.get(t));
                let value = match ty {
                    None => Err(format!("give `{}.{}` first, so that the field's type is known", g.slot, wire::TYPE_KEY)),
                    Some(ty) => match ty.field(f) {
                        Some(t) => literal(t, raw),
                        None => Err(format!("`{}` has no field `{f}`", ty.name)),
                    },
                };
                match value {
                    Ok(v) => Filling::set(fields, f, v, &g.origin),
                    Err(e) => problems.push(format!("{}: {}.{f}: {e}", g.origin, g.slot)),
                }
            }
        }
    }

    let mut bound = vec![];
    for (slot, filled) in program.slots.iter_mut().zip(filling) {
        let (value, origins) = match filled {
            Filling::Text(Some((text, origin))) => (Payload::new(slot.ty.clone(), text.as_bytes()), vec![(String::new(), origin)]),
            Filling::Text(None) => {
                problems.push(format!("{}: required, not provided", slot.name));
                (Payload::text(""), vec![])
            }
            Filling::Fields(fields) => {
                let Some(type_name) = Filling::type_name(&fields) else {
                    problems.push(format!("{}.{}: required, not provided", slot.name, wire::TYPE_KEY));
                    bound.push(Arg::Payload(Payload::text("")));
                    continue;
                };
                let Some(ty) = program.messages.types.get(type_name) else {
                    problems.push(format!("{}: no module in the image declares a message type `{type_name}`", slot.name));
                    bound.push(Arg::Payload(Payload::text("")));
                    continue;
                };
                let missing: Vec<&str> = ty.fields.iter().map(|(n, _)| n.as_str()).filter(|n| !fields.iter().any(|(f, ..)| f == n)).collect();
                for m in &missing {
                    problems.push(format!("{}.{m}: required, not provided", slot.name));
                }
                // Where each field came from, in the type's order.
                let mut origins: Vec<(String, String)> = vec![];
                for name in std::iter::once(wire::TYPE_KEY).chain(ty.fields.iter().map(|(n, _)| n.as_str())) {
                    origins.extend(fields.iter().find(|(f, ..)| f == name).map(|(f, _, o)| (f.clone(), o.clone())));
                }
                let pos = Pos { line: 0, col: 0 };
                let tree = Value::Object(fields.into_iter().map(|(f, v, _)| (Ident { name: f, pos }, v)).collect());
                let bytes = if missing.is_empty() {
                    format::encode(&slot.ty, &program.messages, &tree).unwrap_or_else(|e| {
                        problems.push(format!("{}: {e}", slot.name));
                        vec![]
                    })
                } else {
                    vec![]
                };
                (Payload::new(slot.ty.clone(), bytes), origins)
            }
        };
        slot.value = Some(Arg::Payload(value.clone()));
        slot.origins = origins;
        bound.push(Arg::Payload(value));
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
    use crate::format::{Bson, Format};
    use std::path::Path;

    /// Checks `src`, whose one module, `A`, takes what IMAGE writes to it
    /// and declares `Info`.
    fn program(src: &str) -> Program {
        let a = "export object A { serves { /info write application/bson  /x write */*  /host write text/plain } }";
        let info = "message Info { kind: string, size: i64, mime: string }";
        let header = crate::iface::resolve(&crate::iface::parse(a).unwrap(), &mut |_| Ok(crate::iface::env().clone()), &|_| {
            unreachable!()
        })
        .unwrap();
        let wasm = crate::iface::embed(&wat::parse_str("(module)").unwrap(), &header).unwrap();
        let wasm = crate::iface::embed_schemas::<Bson>(&wasm, &Bson::parse(info, &mut |_| unreachable!()).unwrap()).unwrap();
        let mut loader = crate::iface::Loader::default();
        loader.files.insert("/slots/A.oth".into(), a.into());
        loader.files.insert("/slots/A.messages".into(), info.into());
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
        bind(&mut p, &values).unwrap();

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
        bind(&mut p, &[&info[..], &[whole("a.vals:1:8", "host", "a"), whole("--set", "host", "example.com")]].concat()).unwrap();
        assert_eq!(p.startup[2].body, Some(Arg::Payload(Payload::text("example.com"))));
        assert_eq!(p.slots[2].origins, [(String::new(), "--set".to_string())]);

        let mut p = program("require host : text/plain instance a : A mount /a a write /a/host host");
        let e = bind(&mut p, &[]).unwrap_err().to_string();
        assert!(e.ends_with("host: required, not provided"), "{e}");
    }

    #[test]
    fn a_whole_structured_value_merges_with_its_default() {
        let mut p = program(IMAGE);
        let tree = |src: &str| {
            let mut parser = crate::image::Parser::new(src, false).unwrap();
            parser.value().unwrap()
        };
        bind(&mut p, &[given("v.vals:1:8", "info", None, GivenValue::Literal(tree(r#"{ size: 3, mime: "" }"#)))]).unwrap();
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
        let e = bind(&mut p, &values).unwrap_err().to_string();
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
        let e = bind(&mut p, &[text("--set", "g", "kind", "x")]).unwrap_err().to_string();
        assert!(e.contains("g.kind: give `g.$type` first"), "{e}");
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
        bind(&mut p, &[]).unwrap();
        let e = bind(&mut p, &[text("--set", "x", "y", "z")]).unwrap_err().to_string();
        assert!(e.contains("there is no slot `x`"), "{e}");
    }
}
