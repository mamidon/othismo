//! Binding slots: filling each `require`d message from the image's defaults
//! and the values the host gives, then putting the results wherever the
//! slots are used. See `docs/images.md`.

use crate::check::{self, Arg, Program};
use crate::image::Value;
use crate::messages::FieldType;
use crate::wire::Message;
use anyhow::{Result, bail};
use bson::Document;

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
    /// Raw text, from `--set`: kept as is for a `string` or `ArrayBuffer`
    /// field, and read as a literal for any other.
    Text(String),
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
        inst.startup.iter_mut().filter_map(|s| s.body.as_mut()).for_each(fill);
    }
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

    fn program(src: &str) -> Program {
        check::check(&image::parse(src).unwrap(), Path::new(".")).unwrap()
    }

    const IMAGE: &str = r#"
        require listen : Listen { address: "127.0.0.1:8080" }
        require info : Info { kind: "file" }
        instance net : Environment.Tcp { send listen }
        instance a : A { write /info info  write /x Text { text: "literal" } }
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
        let info = p.instances[1].startup[0].body.as_ref().unwrap().message().unwrap();
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
    fn an_image_without_slots_needs_nothing() {
        let mut p = program("instance cons : Environment.Console");
        bind(&mut p, &[]).unwrap();
        let e = bind(&mut p, &[text("--set", "x", "y", "z")]).unwrap_err().to_string();
        assert!(e.contains("there is no slot `x`"), "{e}");
    }
}
