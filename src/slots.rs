//! Binding slots: filling each `require`d message from the image's defaults
//! and the values the host gives, then putting the results wherever the
//! slots are used. See `docs/images.md`.

use crate::check::{self, Arg, Program};
use crate::image::Value;
use crate::messages::FieldType;
use crate::wire::Message;
use anyhow::{Context, Result, bail};
use bson::Document;
use std::path::Path;

/// One field's value from the host.
#[derive(Debug, Clone, PartialEq)]
pub struct Given {
    /// Where it came from, for errors and `--check`: `prod.vals:3:5`, `--set`.
    pub origin: String,
    pub slot: String,
    pub field: String,
    pub value: GivenValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GivenValue {
    /// A literal, from a value file.
    Literal(Value),
    /// Raw text, from `--set`: kept as is for a `string` or `bytes`
    /// field, and read as a literal for any other.
    Text(String),
}

/// The values in a value file, in order.
pub fn read_values(path: &Path) -> Result<Vec<Given>> {
    let src = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let blocks = crate::image::parse_values(&src).with_context(|| format!("parsing {}", path.display()))?;
    let mut out = vec![];
    for block in blocks {
        for (field, value) in block.fields {
            out.push(Given {
                origin: format!("{}:{}", path.display(), field.pos),
                slot: block.slot.name.clone(),
                field: field.name,
                value: GivenValue::Literal(value),
            });
        }
    }
    Ok(out)
}

/// The value in a `--set slot.field=value` argument.
pub fn parse_set(arg: &str) -> Result<Given> {
    let parsed = arg.split_once('=').and_then(|(key, value)| Some((key.split_once('.')?, value)));
    let Some(((slot, field), value)) = parsed.filter(|((s, f), _)| !s.is_empty() && !f.is_empty()) else {
        bail!("--set {arg}: expected slot.field=value");
    };
    Ok(Given { origin: "--set".into(), slot: slot.into(), field: field.into(), value: GivenValue::Text(value.into()) })
}

/// Fills every slot: the image's defaults first, then `given` in order, a
/// later value replacing an earlier one. Fails, listing every problem, if a
/// value names an unknown slot or field or doesn't fit its field, or if any
/// field is left without a value.
pub fn bind(program: &mut Program, given: &[Given]) -> Result<()> {
    let mut problems = vec![];
    let mut values: Vec<(Document, Vec<(String, String)>)> = program
        .slots
        .iter()
        .map(|s| (s.defaults.clone(), s.defaults.keys().map(|k| (k.clone(), "default".to_string())).collect()))
        .collect();

    for g in given {
        let Some(i) = program.slots.iter().position(|s| s.name == g.slot) else {
            problems.push(format!("{}: there is no slot `{}`", g.origin, g.slot));
            continue;
        };
        let ty = &program.messages[&program.slots[i].type_name];
        let value = match &g.value {
            GivenValue::Literal(v) => Ok(v.clone()),
            GivenValue::Text(text) => match ty.field(&g.field) {
                Some(t) => literal(t, text),
                None => Err(format!("`{}` has no field `{}`", ty.name, g.field)),
            },
        };
        match value.and_then(|v| check::field_bson(ty, &g.field, &v)) {
            Ok(bson) => {
                let (fields, origins) = &mut values[i];
                fields.insert(g.field.clone(), bson);
                origins.retain(|(f, _)| *f != g.field);
                origins.push((g.field.clone(), g.origin.clone()));
            }
            Err(e) => problems.push(format!("{}: {}.{}: {e}", g.origin, g.slot, g.field)),
        }
    }

    let mut bound = vec![];
    for (slot, (fields, origins)) in program.slots.iter_mut().zip(values) {
        let ty = &program.messages[&slot.type_name];
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
        let message = Message { type_name: slot.type_name.clone(), fields: ordered };
        slot.value = Some(message.clone());
        slot.origins = in_order;
        bound.push(message);
    }
    if !problems.is_empty() {
        bail!("the image's slots can't be filled:\n  {}", problems.join("\n  "));
    }

    let fill = |arg: &mut Arg| {
        if let Arg::Slot(i) = *arg {
            *arg = Arg::Message(bound[i].clone());
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
    use crate::image;
    use std::path::Path;

    /// Checks `src`, whose one module, `A`, takes what IMAGE writes to it.
    fn program(src: &str) -> Program {
        let a = "import { Info, Text } from env object A { serves { /info write Info  /x write Text } }";
        let header = crate::iface::resolve(&crate::iface::parse(a).unwrap(), &mut |_| Ok(crate::iface::env().clone()), &|_| {
            unreachable!()
        })
        .unwrap();
        let mut modules = |_: &str| Ok(header.clone());
        check::check(&image::parse(src).unwrap(), Path::new("."), &mut modules).unwrap()
    }

    const IMAGE: &str = r#"
        require listen : Listen { address: "127.0.0.1:8080" }
        require info : Info { kind: "file" }
        instance net : Listener
        instance a : A
        mount /net net
        mount /a a
        control /net listen
        write /a/info info
        write /a/x Text { text: "literal" }
    "#;

    fn given(origin: &str, slot: &str, field: &str, value: GivenValue) -> Given {
        Given { origin: origin.into(), slot: slot.into(), field: field.into(), value }
    }

    fn text(origin: &str, slot: &str, field: &str, value: &str) -> Given {
        given(origin, slot, field, GivenValue::Text(value.into()))
    }

    #[test]
    fn later_values_win_field_by_field() {
        let mut p = program(IMAGE);
        let values = [
            given("a.vals:2:3", "listen", "address", GivenValue::Literal(Value::Str("0.0.0.0:80".into()))),
            text("--set", "info", "size", "42"),
            text("--set", "info", "mime", "text/plain"),
            text("--set", "listen", "address", "0.0.0.0:8080"),
        ];
        bind(&mut p, &values).unwrap();

        let config = p.instances[0].config.as_ref().unwrap().message().unwrap();
        assert_eq!(config.fields, bson::doc! { "address": "0.0.0.0:8080" });
        let info = p.startup[0].body.as_ref().unwrap().message().unwrap();
        assert_eq!(info.fields, bson::doc! { "kind": "file", "size": 42i64, "mime": "text/plain" }, "in declared order");
        let origins = |i: usize| p.slots[i].origins.iter().map(|(f, o)| format!("{f}={o}")).collect::<Vec<_>>();
        assert_eq!(origins(0), ["address=--set"]);
        assert_eq!(origins(1), ["kind=default", "size=--set", "mime=--set"]);
    }

    #[test]
    fn every_problem_is_listed() {
        let mut p = program(IMAGE);
        let values = [
            text("--set", "lisen", "address", "x"),
            text("--set", "listen", "port", "80"),
            text("--set", "info", "size", "big"),
            given("v.vals:1:8", "info", "kind", GivenValue::Literal(Value::Int(1))),
        ];
        let e = bind(&mut p, &values).unwrap_err().to_string();
        assert_eq!(
            e,
            "the image's slots can't be filled:\n  \
             --set: there is no slot `lisen`\n  \
             --set: listen.port: `Listen` has no field `port`\n  \
             --set: info.size: `big` is not a valid i64\n  \
             v.vals:1:8: info.kind: `Info.kind` is string, but was given Int(1)\n  \
             info.size: required, not provided\n  \
             info.mime: required, not provided"
        );
    }

    #[test]
    fn values_come_from_files_and_set() {
        let dir = std::env::temp_dir().join(format!("othismo_values_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prod.vals");
        std::fs::write(&file, "listen {\n  address: \"0.0.0.0:80\"\n}\ninfo { size: 1 }").unwrap();
        let values = read_values(&file).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].origin, format!("{}:2:3", file.display()));
        assert_eq!((values[1].slot.as_str(), values[1].field.as_str()), ("info", "size"));
        assert_eq!(values[1].value, GivenValue::Literal(Value::Int(1)));

        assert_eq!(parse_set("listen.address=0.0.0.0:80=x").unwrap(), text("--set", "listen", "address", "0.0.0.0:80=x"));
        assert_eq!(parse_set("a.b=").unwrap(), text("--set", "a", "b", ""));
        for bad in ["listen=1", "listen.address", ".address=1", "listen.=1"] {
            assert!(parse_set(bad).unwrap_err().to_string().contains("expected slot.field=value"), "{bad}");
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
