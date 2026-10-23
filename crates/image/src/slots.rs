//! Binding slots: filling each of the root image's `require`d values from
//! its default and the values the host gives, then putting the results
//! wherever the slots are used. A structured slot is filled field by field
//! and then encoded by its format. See `docs/images.md`.

use crate::program::{Arg, Program, SlotValue};
use protocol::payload::Payload;
use syntax::image::{Ident, Pos, Value};
use anyhow::{Context, Result, bail};
use format::{Format, Formats, Schemas};
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
    let blocks = syntax::image::parse_values(&src).with_context(|| format!("parsing {}", path.display()))?;
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

    /// A structured value's fields so far, without where they came from.
    fn so_far(fields: &[(String, Value, String)]) -> Vec<(String, Value)> {
        fields.iter().map(|(f, v, _)| (f.clone(), v.clone())).collect()
    }
}

/// Fills every slot: the image's defaults first, then `given` in order, a
/// later value replacing an earlier one, field by field for a structured
/// slot. Fails, listing every problem, if a value names an unknown slot or
/// field or doesn't fit, or if any slot or field is left without a value.
/// A structured slot is filled and encoded by whichever of `formats` is for
/// its content type.
pub fn bind(program: &mut Program, formats: &Formats, given: &[Given]) -> Result<()> {
    let mut problems = vec![];
    let messages = program.messages.clone();
    let format = |ty: &str| -> Option<(&dyn Format, Schemas)> {
        let f = formats.for_type(ty)?;
        Some((f, messages.get(f.content_type()).cloned().unwrap_or_default()))
    };
    let mut filling: Vec<Filling> = program
        .slots
        .iter()
        .map(|s| match (&s.default, s.structured(formats)) {
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
                let (format, schemas) = format(&slot.ty).expect("a structured slot has a format");
                match format.literal(&schemas, &Filling::so_far(fields), f, raw) {
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
                let (format, schemas) = format(&slot.ty).expect("a structured slot has a format");
                let wanted = format.fields(&schemas, &Filling::so_far(&fields));
                let missing: Vec<&str> = wanted.iter().map(String::as_str).filter(|n| !fields.iter().any(|(f, ..)| f == n)).collect();
                for m in &missing {
                    problems.push(format!("{}.{m}: required, not provided", slot.name));
                }
                // Where each field came from, in the type's order.
                let mut origins: Vec<(String, String)> = vec![];
                for name in &wanted {
                    origins.extend(fields.iter().find(|(f, ..)| f == name).map(|(f, _, o)| (f.clone(), o.clone())));
                }
                let pos = Pos { line: 0, col: 0 };
                let tree = Value::Object(fields.into_iter().map(|(f, v, _)| (Ident { name: f, pos }, v)).collect());
                let bytes = if missing.is_empty() {
                    format.encode(&schemas, &tree).unwrap_or_else(|e| {
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
