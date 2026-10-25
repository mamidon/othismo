//! The BSON format, for `application/bson`: a payload is a BSON document
//! whose first key, `$type`, names its message type. Its schemas are
//! message types, written in a `.oth` file's `schemas application/bson`
//! block.
//! It is the format the SDK supports, and the protocol knows nothing of it.

pub mod bindings;
pub mod messages;
mod parse;
pub mod wire;

use anyhow::{Result, bail};
use bson::{Bson as B, Document};
use format::{Bindings, FieldDoc, Format, MessageDoc, Schemas, json_string};
use messages::{EnumType, FieldType, MessageType};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;
use syntax::image_definition::{Ident, Parser, Value};
use wire::Message;

/// The content type of a BSON payload.
pub const BSON: &str = "application/bson";

/// BSON messages, as a [`Format`].
pub struct Bson;

/// A set of message types and enums, and the ones declared where it was
/// read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Messages {
    pub types: BTreeMap<String, MessageType>,
    #[serde(default)]
    pub enums: BTreeMap<String, EnumType>,
    /// The ones the file declares itself, in order; the rest it imports.
    pub declared: Vec<String>,
}

impl Messages {
    /// Whether it has a message type or an enum called `name`.
    pub fn has(&self, name: &str) -> bool {
        self.types.contains_key(name) || self.enums.contains_key(name)
    }

    /// `name`, and every type its fields name, however deeply, each once.
    pub fn closure(&self, name: &str) -> Vec<String> {
        let mut out = vec![name.to_string()];
        let mut i = 0;
        while i < out.len() {
            for f in self.types.get(&out[i]).map_or(&[][..], |t| &t.fields[..]) {
                for n in f.ty.names() {
                    if !out.iter().any(|o| o == n) {
                        out.push(n.to_string());
                    }
                }
            }
            i += 1;
        }
        out
    }
}

impl Bson {
    /// The message types in `schemas`, which this format wrote.
    pub fn messages(schemas: &Schemas) -> Messages {
        if schemas.0.is_empty() {
            return Messages::default();
        }
        let doc = Document::from_reader(&schemas.0[..]).expect("BSON's schemas are its own");
        bson::deserialize_from_document(doc).expect("BSON's schemas are its own")
    }

    /// `messages`, as schemas.
    pub fn schemas(messages: &Messages) -> Schemas {
        let mut out = vec![];
        bson::serialize_to_document(messages).expect("message types serialize").to_writer(&mut out).expect("into a Vec");
        Schemas(out)
    }

    pub fn parse_messages(src: &str, import: &mut dyn FnMut(&str) -> Result<Messages>) -> Result<Messages> {
        parse::messages(src, import)
    }

    pub fn merge_messages(all: &mut Messages, from: &mut BTreeMap<String, String>, more: &Messages, origin: &str) -> Result<()> {
        for (name, e) in &more.enums {
            match all.enums.get(name) {
                Some(prev) if prev.values != e.values => {
                    let before = from.get(name).map_or("another file", String::as_str);
                    bail!("`{name}` is declared one way by {before} and another by {origin}; an image can't use both")
                }
                Some(_) => {}
                None => {
                    all.enums.insert(name.clone(), e.clone());
                    from.insert(name.clone(), origin.to_string());
                }
            }
        }
        for (name, ty) in &more.types {
            match all.types.get(name) {
                Some(prev) if prev.fields != ty.fields => {
                    let before = from.get(name).map_or("another file", String::as_str);
                    bail!("`{name}` is declared one way by {before} and another by {origin}; an image can't use both")
                }
                Some(_) => {}
                None => {
                    all.types.insert(name.clone(), ty.clone());
                    from.insert(name.clone(), origin.to_string());
                }
            }
        }
        Ok(())
    }

    /// `value`, a message of type `type_name`, encoded: every field given.
    pub fn encode_message(schemas: &Messages, type_name: &str, value: &Value) -> Result<Vec<u8>, String> {
        let fields = fields(schemas, type_name, value)?;
        let ty = &schemas.types[type_name];
        if let Some(missing) = ty.fields.iter().find(|f| !f.optional && !fields.contains_key(&f.name)) {
            return Err(format!("`{type_name}` is missing field `{}`", missing.name));
        }
        Ok(Message { type_name: type_name.to_string(), fields }.encode())
    }

    pub fn document_messages(schemas: &Messages, h: &str) -> String {
        let mut out = String::new();
        let types: Vec<&MessageType> = schemas.declared.iter().filter_map(|t| schemas.types.get(t)).collect();
        let enums: Vec<&EnumType> = schemas.declared.iter().filter_map(|t| schemas.enums.get(t)).collect();
        if !types.is_empty() {
            writeln!(out, "{h} Messages\n\nPayloads of `{}`, each named by its `$type`.\n", BSON).unwrap();
        }
        for ty in types {
            writeln!(out, "{h}# `{}`\n", ty.name).unwrap();
            if !ty.doc.is_empty() {
                writeln!(out, "{}\n", ty.doc).unwrap();
            }
            if ty.fields.is_empty() {
                out.push_str("No fields.\n\n");
                continue;
            }
            out.push_str("| Field | Type | |\n|---|---|---|\n");
            for (f, doc) in ty.fields.iter().zip(&ty.field_docs) {
                let doc = doc.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|");
                let optional = if f.optional { ", optional" } else { "" };
                writeln!(out, "| `{}` | {}{optional} | {doc} |", f.name, f.ty).unwrap();
            }
            out.push('\n');
        }
        if !enums.is_empty() {
            writeln!(out, "{h} Enums\n\nCarried as the name of their value, a string.\n").unwrap();
        }
        for e in enums {
            writeln!(out, "{h}# `{}`\n", e.name).unwrap();
            if !e.doc.is_empty() {
                writeln!(out, "{}\n", e.doc).unwrap();
            }
            out.push_str("| Value | |\n|---|---|\n");
            for (v, doc) in e.values.iter().zip(&e.value_docs) {
                let doc = doc.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|");
                writeln!(out, "| `{v}` | {doc} |").unwrap();
            }
            out.push('\n');
        }
        out
    }
}

impl Format for Bson {
    fn content_type(&self) -> &'static str {
        BSON
    }

    fn section(&self) -> &'static str {
        "othismo.schema.bson"
    }

    fn load(&self, bytes: &[u8]) -> Result<Schemas> {
        let messages: Messages = bson::deserialize_from_document(Document::from_reader(bytes)?)?;
        Ok(Bson::schemas(&messages))
    }

    fn parse(&self, src: &str, import: &mut dyn FnMut(&str) -> Result<Schemas>) -> Result<Schemas> {
        Ok(Bson::schemas(&Bson::parse_messages(src, &mut |p| import(p).map(|s| Bson::messages(&s)))?))
    }

    fn merge(&self, all: &mut Schemas, from: &mut BTreeMap<String, String>, more: &Schemas, origin: &str) -> Result<()> {
        let mut types = Bson::messages(all);
        Bson::merge_messages(&mut types, from, &Bson::messages(more), origin)?;
        *all = Bson::schemas(&types);
        Ok(())
    }

    fn same(&self, a: &Schemas, b: &Schemas) -> bool {
        let (a, b) = (Bson::messages(a), Bson::messages(b));
        a.types == b.types && a.enums == b.enums
    }

    fn encode(&self, schemas: &Schemas, ty: &str, value: &Value) -> Result<Vec<u8>, String> {
        Bson::encode_message(&Bson::messages(schemas), ty, value)
    }

    fn check(&self, schemas: &Schemas, ty: &str, value: &Value) -> Result<(), String> {
        fields(&Bson::messages(schemas), ty, value).map(|_| ())
    }

    fn fields(&self, schemas: &Schemas, ty: &str) -> Result<Vec<(String, bool)>, String> {
        let messages = Bson::messages(schemas);
        Ok(message_type(&messages, ty)?.fields.iter().map(|f| (f.name.clone(), !f.optional)).collect())
    }

    fn literal(&self, schemas: &Schemas, ty: &str, field: &str, raw: &str) -> Result<Value, String> {
        let messages = Bson::messages(schemas);
        let ty = message_type(&messages, ty)?;
        match ty.field(field) {
            Some(f) => literal(&messages, &f.ty, raw),
            None => Err(format!("`{}` has no field `{field}`", ty.name)),
        }
    }

    fn show(&self, bytes: &[u8]) -> Option<Vec<(String, String)>> {
        let m = Message::decode(bytes).ok()?;
        Some(m.fields.iter().map(|(f, v)| (f.clone(), json_value(v))).collect())
    }

    fn describe(&self, schemas: &Schemas, ty: &str) -> Result<MessageDoc, String> {
        let messages = Bson::messages(schemas);
        let t = message_type(&messages, ty)?;
        let fields = t
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| FieldDoc {
                name: f.name.clone(),
                ty: if f.optional { format!("{}, optional", f.ty) } else { f.ty.to_string() },
                examples: examples(&messages, &f.ty),
                doc: t.field_docs.get(i).cloned().unwrap_or_default(),
            })
            .collect();
        Ok(MessageDoc { name: t.name.clone(), doc: t.doc.clone(), fields })
    }

    /// Only message types: an enum can't be a payload.
    fn declared(&self, schemas: &Schemas) -> Vec<String> {
        let m = Bson::messages(schemas);
        m.declared.into_iter().filter(|t| m.types.contains_key(t)).collect()
    }

    fn bindings(&self, schemas: &Schemas, types: &[String], library: &str) -> Option<Bindings> {
        let m = Bson::messages(schemas);
        let enums = m.declared.iter().any(|t| m.enums.contains_key(t));
        (!types.is_empty() || enums).then(|| bindings::assemblyscript(&m, types, library))
    }

    fn document(&self, schemas: &Schemas, h: &str) -> String {
        Bson::document_messages(&Bson::messages(schemas), h)
    }
}

/// Literals a field of type `t` could be given, to start from: every value
/// of an enum, and an empty one of the rest. `any` has none, since it
/// starts with a type's name.
fn examples(schemas: &Messages, t: &FieldType) -> Vec<String> {
    match t {
        FieldType::Bool => vec!["true".into(), "false".into()],
        FieldType::String | FieldType::Bytes => vec!["\"\"".into()],
        FieldType::F32 | FieldType::F64 => vec!["0.0".into()],
        FieldType::Time => vec!["\"1970-01-01T00:00:00Z\"".into()],
        FieldType::Uuid => vec!["\"00000000-0000-0000-0000-000000000000\"".into()],
        FieldType::Named(n) => match schemas.enums.get(n) {
            Some(e) => e.values.iter().map(|v| json_string(v)).collect(),
            None => vec!["{}".into()],
        },
        FieldType::List(_) => vec!["[]".into()],
        FieldType::Map(_) => vec!["{}".into()],
        FieldType::Any => vec![],
        _ => vec!["0".into()],
    }
}

/// Message type `name`, which some module or file in the image declares.
fn message_type<'a>(schemas: &'a Messages, name: &str) -> Result<&'a MessageType, String> {
    schemas.types.get(name).ok_or_else(|| format!("no module in the image declares a message type `{name}`"))
}

/// Reads `text` as a literal for a field of type `t`.
fn literal(schemas: &Messages, t: &FieldType, text: &str) -> std::result::Result<Value, String> {
    let bad = || format!("`{text}` is not a valid {t}");
    Ok(match t {
        FieldType::String | FieldType::Bytes | FieldType::Uuid => Value::Str(text.to_string()),
        FieldType::Named(n) if schemas.enums.contains_key(n) => Value::Str(text.to_string()),
        FieldType::Time => text.parse().map(Value::Int).unwrap_or_else(|_| Value::Str(text.to_string())),
        FieldType::Bool => match text {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(bad()),
        },
        FieldType::F32 | FieldType::F64 => Value::Float(text.parse().map_err(|_| bad())?),
        FieldType::Named(_) | FieldType::List(_) | FieldType::Map(_) | FieldType::Any => {
            let mut p = Parser::new(text, false).map_err(|e| e.to_string())?;
            let v = p.value().map_err(|e| e.to_string())?;
            if *p.peek() != syntax::image_definition::Tok::Eof {
                return Err(format!("`{text}` is more than one value"));
            }
            v
        }
        _ => Value::Int(text.parse().map_err(|_| bad())?),
    })
}

/// A BSON value as JSON-ish text.
pub fn json_value(v: &B) -> String {
    match v {
        B::String(s) => json_string(s),
        B::Boolean(b) => b.to_string(),
        B::Int32(i) => i.to_string(),
        B::Int64(i) => i.to_string(),
        B::Double(x) if x.is_finite() => format!("{x:?}"),
        B::Double(x) => json_string(&x.to_string()),
        B::Binary(b) if b.subtype == bson::spec::BinarySubtype::Uuid && b.bytes.len() == 16 => json_string(&wire::show_uuid(&b.bytes)),
        B::DateTime(t) => t.try_to_rfc3339_string().map_or_else(|_| t.timestamp_millis().to_string(), |s| json_string(&s)),
        B::Binary(b) => match std::str::from_utf8(&b.bytes) {
            Ok(s) => json_string(s),
            Err(_) => format!("<{} bytes>", b.bytes.len()),
        },
        B::Array(items) => format!("[{}]", items.iter().map(json_value).collect::<Vec<_>>().join(", ")),
        B::Document(d) => {
            let fields: Vec<String> = d.iter().map(|(k, v)| format!("{}: {}", json_string(k), json_value(v))).collect();
            format!("{{{}}}", fields.join(", "))
        }
        other => json_string(&other.to_string()),
    }
}


/// The fields of `value`, a message of type `type_name`, as given, each
/// checked against the type. Fields left out are fine here.
pub fn fields(schemas: &Messages, type_name: &str, value: &Value) -> Result<Document, String> {
    let Value::Object(given) = value else { return Err(format!("a message is written `{type_name} {{ <field>: <value>, ... }}`")) };
    let ty = message_type(schemas, type_name)?;
    if let Some((k, _)) = given.iter().find(|(k, _)| k.name == wire::TYPE_KEY) {
        return Err(format!("{}: the message's type is named before its braces, `{type_name} {{ ... }}`, not with `{}`", k.pos, wire::TYPE_KEY));
    }
    document(schemas, ty, given, type_name)
}

/// The fields `given`, of a value of type `ty`, each checked against the
/// type, in declared order, as the module's own encoder writes them. `at`
/// says where the value is, for errors.
fn document(schemas: &Messages, ty: &MessageType, given: &[(Ident, Value)], at: &str) -> Result<Document, String> {
    let mut fields = Document::new();
    for (name, v) in given {
        if fields.contains_key(&name.name) {
            return Err(format!("field `{}` given twice", name.name));
        }
        let Some(f) = ty.field(&name.name) else {
            return Err(format!("`{at}` has no field `{}`", name.name));
        };
        fields.insert(name.name.clone(), to_bson(schemas, &f.ty, v, &format!("{at}.{}", name.name))?);
    }
    let mut ordered = Document::new();
    for f in &ty.fields {
        if let Some(v) = fields.remove(&f.name) {
            ordered.insert(f.name.clone(), v);
        }
    }
    Ok(ordered)
}

/// The fields `given`, a whole value of message type `name`: they must
/// leave out none but its optional fields.
fn whole(schemas: &Messages, name: &str, given: &[(Ident, Value)], at: &str) -> Result<Document, String> {
    let ty = message_type(schemas, name)?;
    let doc = document(schemas, ty, given, at)?;
    if let Some(missing) = ty.fields.iter().find(|f| !f.optional && !doc.contains_key(&f.name)) {
        return Err(format!("`{at}` is missing field `{}`", missing.name));
    }
    Ok(doc)
}

/// The BSON for `value`, if it fits field type `t`. `at` says where it is.
fn to_bson(schemas: &Messages, t: &FieldType, value: &Value, at: &str) -> std::result::Result<B, String> {
    Ok(match (t, value) {
        (FieldType::String, Value::Str(s)) => B::String(s.clone()),
        (FieldType::Bool, Value::Bool(b)) => B::Boolean(*b),
        (FieldType::F32 | FieldType::F64, Value::Float(x)) => B::Double(*x),
        (FieldType::F32 | FieldType::F64, Value::Int(i)) => B::Double(*i as f64),
        (FieldType::Bytes, Value::Str(s)) => wire::bytes(s.as_bytes().to_vec()),
        (FieldType::Time, Value::Str(s)) => match bson::DateTime::parse_rfc3339_str(s) {
            Ok(t) => B::DateTime(t),
            Err(_) => return Err(format!("`{at}` is a time, written as in RFC 3339, `\"2026-10-23T12:00:00Z\"`, or as milliseconds since 1970; {s:?} is neither")),
        },
        (FieldType::Time, Value::Int(ms)) => match i64::try_from(*ms) {
            Ok(ms) => B::DateTime(bson::DateTime::from_millis(ms)),
            Err(_) => return Err(format!("{ms} does not fit in a time")),
        },
        (FieldType::Uuid, Value::Str(s)) => match wire::parse_uuid(s) {
            Some(id) => wire::uuid(id),
            None => return Err(format!("`{at}` is a uuid, written `\"123e4567-e89b-12d3-a456-426614174000\"`; {s:?} isn't one")),
        },
        (t, Value::Int(i)) if t.int_range().is_some() => {
            let (lo, hi) = t.int_range().unwrap();
            if !(lo..=hi).contains(i) {
                return Err(format!("{i} does not fit in {t}"));
            }
            wire::int_to_bson(t, *i)
        }
        (FieldType::Named(n), v) if schemas.enums.contains_key(n) => {
            let e = &schemas.enums[n];
            match v {
                Value::Str(s) if e.values.contains(s) => B::String(s.clone()),
                v => {
                    let values: Vec<String> = e.values.iter().map(|v| format!("{v:?}")).collect();
                    return Err(format!("`{at}` is a `{n}`, one of {}, but was given {v}", values.join(", ")));
                }
            }
        }
        (FieldType::Named(n), Value::Object(given)) => B::Document(whole(schemas, n, given, at)?),
        (FieldType::Named(n), Value::Typed(t, v)) => match &**v {
            _ if t.name != *n => return Err(format!("`{at}` is a `{n}`, not a `{}`", t.name)),
            Value::Object(given) => B::Document(whole(schemas, n, given, at)?),
            v => return Err(format!("`{at}` is a `{n}`, written `{{ <field>: <value>, ... }}`, but was given {v}")),
        },
        (FieldType::Any, Value::Typed(t, v)) => {
            let Value::Object(given) = &**v else { return Err(format!("`{at}`'s {} is written `{} {{ <field>: <value>, ... }}`", t.name, t.name)) };
            let mut doc = Document::new();
            doc.insert(wire::TYPE_KEY, t.name.clone());
            doc.extend(whole(schemas, &t.name, given, at)?);
            B::Document(doc)
        }
        (FieldType::Any, v) => return Err(format!("`{at}` is any message, written with its type, `Name {{ ... }}`, but was given {v}")),
        (FieldType::List(e), Value::List(items)) => {
            let items = items.iter().enumerate().map(|(i, v)| to_bson(schemas, e, v, &format!("{at}[{i}]")));
            B::Array(items.collect::<std::result::Result<_, _>>()?)
        }
        (FieldType::Map(e), Value::Object(entries)) => {
            let mut doc = Document::new();
            for (k, v) in entries {
                if k.name.contains('\0') {
                    return Err(format!("`{at}`'s key {:?} has a NUL in it, which BSON can't carry", k.name));
                }
                if doc.contains_key(&k.name) {
                    return Err(format!("`{at}` has the key {:?} twice", k.name));
                }
                doc.insert(k.name.clone(), to_bson(schemas, e, v, &format!("{at}[{}]", json_string(&k.name)))?);
            }
            B::Document(doc)
        }
        (FieldType::Named(n), v) => return Err(format!("`{at}` is a `{n}`, written `{{ <field>: <value>, ... }}`, but was given {v}")),
        (t, v) => return Err(format!("`{at}` is {t}, but was given {v}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::content_type::ContentType;

    fn messages(src: &str) -> Messages {
        Bson::parse_messages(src, &mut |p| bail!("no {p} here")).unwrap()
    }

    fn value(src: &str) -> Value {
        let mut p = Parser::new(src, false).unwrap();
        p.value().unwrap()
    }

    #[test]
    fn a_message_type_is_described_field_by_field() {
        let schemas = Bson.parse("## Configures it.\nmessage Greeting {\n  ## Before each name.\n  prefix: string\n  excited: bool\n  times: u8\n}", &mut |p| bail!("no {p} here")).unwrap();
        let d = Bson.describe(&schemas, "Greeting").unwrap();
        assert_eq!(d.doc, "Configures it.");
        let fields: Vec<(&str, &str, Vec<&str>, &str)> =
            d.fields.iter().map(|f| (f.name.as_str(), f.ty.as_str(), f.examples.iter().map(String::as_str).collect(), f.doc.as_str())).collect();
        assert_eq!(fields, [
            ("prefix", "string", vec!["\"\""], "Before each name."),
            ("excited", "bool", vec!["true", "false"], ""),
            ("times", "u8", vec!["0"], ""),
        ]);
        assert!(Bson.describe(&schemas, "Nope").is_err());

        let schemas = Bson::schemas(&messages(&format!("{NESTED} {COLORS} message All {{ at: time, p?: Person, c: Color, l: [u8], m: {{u8}}, a: any }}")));
        let d = Bson.describe(&schemas, "All").unwrap();
        let fields: Vec<(&str, Vec<&str>)> = d.fields.iter().map(|f| (f.ty.as_str(), f.examples.iter().map(String::as_str).collect())).collect();
        assert_eq!(fields, [
            ("time", vec![r#""1970-01-01T00:00:00Z""#]),
            ("Person, optional", vec!["{}"]),
            ("Color", vec![r#""red""#, r#""green""#, r#""blue""#]),
            ("[u8]", vec!["[]"]),
            ("{u8}", vec!["{}"]),
            ("any", vec![]),
        ]);
    }

    const GREETING: &str = r#"
        ## Configures a Greeter.
        message Greeting {
          ## Put before each name.
          prefix: string
          excited: bool
        }
        message Odd { a: u8, b: f32, c: u64, d: bytes }
    "#;

    #[test]
    fn schemas_declare_message_types() {
        let m = messages(GREETING);
        assert_eq!(m.declared, ["Greeting", "Odd"]);
        let g = &m.types["Greeting"];
        assert_eq!((g.doc.as_str(), g.field_docs[0].as_str()), ("Configures a Greeter.", "Put before each name."));
        let err = |src: &str| format!("{:#}", Bson::parse_messages(src, &mut |p| bail!("no {p} here")).unwrap_err());
        assert!(err("message A { x: Strin }").contains("1:16: `Strin` is not a field type, nor a message type declared or imported here"));
        assert!(err("message A { x: i32 x: i32 }").contains("two fields called `x`"));
        assert!(err("message A {} message A {}").contains("`A` is declared twice"));
        assert!(err("message A {} enum A { a }").contains("`A` is declared twice"));
        assert!(err("object A {}").contains("expected `import`, `message` or `enum`"));
        assert!(err(r#"import { A } from "./a.oth""#).contains("no ./a.oth here"));

        let mut import = |_: &str| Ok(messages(GREETING));
        let m = Bson::parse_messages(r#"import { Greeting } from "./g.oth" message Mine { g: string }"#, &mut import).unwrap();
        assert_eq!(m.declared, ["Mine"], "imported types have their own bindings");
        assert!(m.types.contains_key("Greeting"));
        let e = format!("{:#}", Bson::parse_messages(r#"import { Greeting as G } from "./g.oth""#, &mut import).unwrap_err());
        assert!(e.contains("can't be renamed"), "{e}");
    }

    const NESTED: &str = r#"
        message Record { owner: Person, note: string }
        message Person { name: string, age: u8 }
    "#;

    #[test]
    fn fields_may_be_messages() {
        let m = messages(NESTED);
        assert_eq!(m.types["Record"].fields[0].ty, FieldType::Named("Person".into()), "named before it is declared");
        assert_eq!(m.closure("Record"), ["Record", "Person"]);
        let bytes = Bson::encode_message(&m, "Record", &value(r#"{ owner: { age: 3, name: "Al" }, note: "n" }"#)).unwrap();
        let msg = Message::decode(&bytes).unwrap();
        let owner = msg.fields.get_document("owner").unwrap();
        assert_eq!(owner.keys().collect::<Vec<_>>(), ["name", "age"], "in declared order, and without a $type");
        assert_eq!(Bson.show(&bytes).unwrap()[0], ("owner".into(), r#"{"name": "Al", "age": 3}"#.into()));
        let err = |src: &str| Bson::encode_message(&m, "Record", &value(src)).unwrap_err();
        assert!(err(r#"{ owner: { name: "Al" }, note: "n" }"#).contains("`Record.owner` is missing field `age`"));
        assert!(err(r#"{ owner: { name: "Al", age: 300 }, note: "n" }"#).contains("300 does not fit in u8"));
        assert!(err(r#"{ owner: { name: "Al", age: 3, x: 1 }, note: "n" }"#).contains("`Record.owner` has no field `x`"));
        assert!(err(r#"{ owner: "Al", note: "n" }"#).contains("`Record.owner` is a `Person`, written `{ <field>: <value>, ... }`, but was given \"Al\""));
        let schemas = Bson::schemas(&m);
        assert_eq!(Bson.literal(&schemas, "Record", "owner", r#"{ name: "Al", age: 3 }"#).map(|v| v.to_string()), Ok(r#"{ name: "Al", age: 3 }"#.into()));
        assert!(Bson.literal(&schemas, "Record", "owner", "{} {}").unwrap_err().contains("more than one value"));
    }

    #[test]
    fn fields_may_be_lists() {
        let m = messages(&format!("{NESTED} message Lists {{ names: [string], grid: [[u8]], people: [Person] }}"));
        assert_eq!(m.types["Lists"].fields[1].ty.to_string(), "[[u8]]");
        assert_eq!(m.closure("Lists"), ["Lists", "Person"]);
        let bytes = Bson::encode_message(&m, "Lists", &value(r#"{ names: ["a", "b"], grid: [[1], []], people: [{ name: "Al", age: 3 }] }"#)).unwrap();
        let shown = Bson.show(&bytes).unwrap();
        assert_eq!(shown[0].1, r#"["a", "b"]"#);
        assert_eq!(shown[1].1, "[[1], []]");
        assert_eq!(shown[2].1, r#"[{"name": "Al", "age": 3}]"#);
        let err = |src: &str| Bson::encode_message(&m, "Lists", &value(src)).unwrap_err();
        assert!(err(r#"{ names: "a", grid: [], people: [] }"#).contains("`Lists.names` is [string], but was given \"a\""));
        assert!(err(r#"{ names: [], grid: [[1, 256]], people: [] }"#).contains("256 does not fit in u8"));
        assert!(err(r#"{ names: [], grid: [], people: [{ name: "Al" }] }"#).contains("`Lists.people[0]` is missing field `age`"));
        assert_eq!(Bson.literal(&Bson::schemas(&m), "Lists", "names", r#"["x"]"#), Ok(Value::List(vec![Value::Str("x".into())])));
        let e = format!("{:#}", Bson::parse_messages("message A { x: [Nope] }", &mut |p| bail!("no {p} here")).unwrap_err());
        assert!(e.contains("1:17: `Nope` is not a field type"), "{e}");
    }

    #[test]
    fn fields_may_be_maps() {
        let m = messages(&format!("{NESTED} message Maps {{ env: {{string}}, by_name: {{Person}}, groups: {{[u8]}} }}"));
        assert_eq!(m.types["Maps"].fields[2].ty.to_string(), "{[u8]}");
        assert_eq!(m.closure("Maps"), ["Maps", "Person"]);
        let src = r#"{ env: { HOME: "/", "Content-Type": "x" }, by_name: { al: { name: "Al", age: 3 } }, groups: { a: [1], b: [] } }"#;
        let shown = Bson.show(&Bson::encode_message(&m, "Maps", &value(src)).unwrap()).unwrap();
        assert_eq!(shown[0].1, r#"{"HOME": "/", "Content-Type": "x"}"#, "in the order given");
        assert_eq!(shown[1].1, r#"{"al": {"name": "Al", "age": 3}}"#);
        assert_eq!(shown[2].1, r#"{"a": [1], "b": []}"#);
        let err = |src: &str| Bson::encode_message(&m, "Maps", &value(src)).unwrap_err();
        assert!(err(r#"{ env: [], by_name: {}, groups: {} }"#).contains("`Maps.env` is {string}, but was given []"));
        assert!(err(r#"{ env: { a: "x", a: "y" }, by_name: {}, groups: {} }"#).contains("`Maps.env` has the key \"a\" twice"));
        assert!(err(r#"{ env: {}, by_name: { al: { name: "Al" } }, groups: {} }"#).contains("`Maps.by_name[\"al\"]` is missing field `age`"));
        assert!(err(r#"{ env: {}, by_name: {}, groups: { a: [300] } }"#).contains("300 does not fit in u8"));
        assert!(err(&format!("{{ env: {{ \"a{}\": \"x\" }}, by_name: {{}}, groups: {{}} }}", '\0')).contains("has a NUL in it"));
    }

    #[test]
    fn fields_may_be_optional() {
        let m = messages(&format!("{NESTED} message Opt {{ nick?: string, n?: i32, p?: Person, must: bool }}"));
        assert!(m.types["Opt"].fields[0].optional && !m.types["Opt"].fields[3].optional);
        assert_ne!(m.types["Opt"], messages(&format!("{NESTED} message Opt {{ nick: string, n?: i32, p?: Person, must: bool }}")).types["Opt"], "optional is part of the type");
        let bytes = Bson::encode_message(&m, "Opt", &value("{ must: true }")).unwrap();
        assert_eq!(Message::decode(&bytes).unwrap().fields.keys().collect::<Vec<_>>(), ["must"], "left out, not null");
        let bytes = Bson::encode_message(&m, "Opt", &value(r#"{ must: true, n: 2, nick: "x" }"#)).unwrap();
        assert_eq!(Message::decode(&bytes).unwrap().fields.keys().collect::<Vec<_>>(), ["nick", "n", "must"]);
        assert!(Bson::encode_message(&m, "Opt", &value("{ n: 2 }")).unwrap_err().contains("missing field `must`"));
        let fields = Bson.fields(&Bson::schemas(&m), "Opt").unwrap();
        assert_eq!(fields, [("nick".into(), false), ("n".into(), false), ("p".into(), false), ("must".into(), true)]);
        let doc = Bson::document_messages(&m, "##");
        assert!(doc.contains("| `nick` | string, optional |"), "{doc}");
        let e = format!("{:#}", Bson::parse_messages("message A { x? i32 }", &mut |p| bail!("no {p} here")).unwrap_err());
        assert!(e.contains("1:16: expected `:`, found `i32`"), "{e}");
    }

    #[test]
    fn fields_may_be_times_and_uuids() {
        let m = messages("message T { at: time, id: uuid, when?: time }");
        let src = r#"{ at: "2026-10-23T12:34:56.789Z", id: "123e4567-e89b-12d3-a456-426614174000", when: 1000 }"#;
        let bytes = Bson::encode_message(&m, "T", &value(src)).unwrap();
        let msg = Message::decode(&bytes).unwrap();
        assert_eq!(msg.fields.get_datetime("at").unwrap().timestamp_millis(), 1_792_758_896_789);
        let id = msg.fields.get_binary_generic("id");
        assert!(id.is_err(), "not the generic subtype");
        let shown = Bson.show(&bytes).unwrap();
        assert_eq!(shown[0].1, r#""2026-10-23T12:34:56.789Z""#);
        assert_eq!(shown[1].1, r#""123e4567-e89b-12d3-a456-426614174000""#);
        assert_eq!(shown[2].1, r#""1970-01-01T00:00:01Z""#);
        let err = |src: &str| Bson::encode_message(&m, "T", &value(src)).unwrap_err();
        assert!(err(r#"{ at: "yesterday", id: "123e4567-e89b-12d3-a456-426614174000" }"#).contains("`T.at` is a time, written as in RFC 3339"));
        assert!(err(r#"{ at: 0, id: "nope" }"#).contains("`T.id` is a uuid"));
        assert!(err(r#"{ at: 99999999999999999999, id: "123e4567-e89b-12d3-a456-426614174000" }"#).contains("does not fit in a time"));
        let schemas = Bson::schemas(&m);
        assert_eq!(Bson.literal(&schemas, "T", "at", "5"), Ok(Value::Int(5)));
        assert_eq!(Bson.literal(&schemas, "T", "at", "2026-10-23T00:00:00Z"), Ok(Value::Str("2026-10-23T00:00:00Z".into())));
    }

    const COLORS: &str = r#"
        ## A color.
        enum Color {
          ## The first.
          red, green
          blue
        }
        message Paint { color: Color, palette: [Color], fav?: Color }
    "#;

    #[test]
    fn fields_may_be_enums() {
        let m = messages(COLORS);
        assert_eq!(m.enums["Color"].values, ["red", "green", "blue"]);
        assert_eq!(m.declared, ["Color", "Paint"]);
        assert_eq!(Bson.declared(&Bson::schemas(&m)), ["Paint"], "an enum is not a payload");
        assert_eq!(m.closure("Paint"), ["Paint", "Color"]);
        let bytes = Bson::encode_message(&m, "Paint", &value(r#"{ color: "red", palette: ["blue", "red"] }"#)).unwrap();
        assert_eq!(Bson.show(&bytes).unwrap()[1].1, r#"["blue", "red"]"#);
        let err = Bson::encode_message(&m, "Paint", &value(r#"{ color: "pink", palette: [] }"#)).unwrap_err();
        assert!(err.contains(r#"`Paint.color` is a `Color`, one of "red", "green", "blue", but was given "pink""#), "{err}");
        assert_eq!(Bson.literal(&Bson::schemas(&m), "Paint", "color", "green"), Ok(Value::Str("green".into())));
        let doc = Bson::document_messages(&m, "##");
        assert!(doc.contains("## Enums\n\nCarried as the name of their value, a string.\n\n### `Color`\n\nA color.\n\n| Value | |\n|---|---|\n| `red` | The first. |\n| `green` |  |\n"), "{doc}");
        let err = |src: &str| format!("{:#}", Bson::parse_messages(src, &mut |p| bail!("no {p} here")).unwrap_err());
        assert!(err("enum E {}").contains("enum `E` has no values"));
        assert!(err("enum E { a, a }").contains("`E` has two values called `a`"));

        let mut import = |_: &str| Ok(messages(COLORS));
        let m = Bson::parse_messages(r#"import { Paint } from "./c.oth""#, &mut import).unwrap();
        assert!(m.enums.contains_key("Color"), "an enum comes with the types that name it");
        let e = format!("{:#}", Bson::parse_messages(r#"enum Color { red } import { Paint } from "./c.oth""#, &mut import).unwrap_err());
        assert!(e.contains("not the `Color` already here"), "{e}");
        let mut all = Bson::schemas(&messages("enum Color { red }"));
        let e = Bson.merge(&mut all, &mut BTreeMap::new(), &Bson::schemas(&messages(COLORS)), "C.oth").unwrap_err().to_string();
        assert!(e.contains("`Color` is declared one way by another file and another by C.oth"), "{e}");
    }

    #[test]
    fn fields_may_be_any_message() {
        let m = messages(&format!("{NESTED} message Crate {{ thing: any, things: [any], maybe?: any, p: Person }}"));
        let src = r#"{ thing: Person { name: "Al", age: 3 }, things: [Record { owner: { name: "Bo", age: 4 }, note: "n" }], p: Person { name: "Cy", age: 5 } }"#;
        let bytes = Bson::encode_message(&m, "Crate", &value(src)).unwrap();
        let msg = Message::decode(&bytes).unwrap();
        let thing = msg.fields.get_document("thing").unwrap();
        assert_eq!(thing.keys().collect::<Vec<_>>(), ["$type", "name", "age"], "its type first, as a payload's");
        let shown = Bson.show(&bytes).unwrap();
        assert_eq!(shown[1].1, r#"[{"$type": "Record", "owner": {"name": "Bo", "age": 4}, "note": "n"}]"#);
        assert_eq!(shown[2].1, r#"{"name": "Cy", "age": 5}"#, "a message field may name its type, but doesn't carry it");
        let err = |src: &str| Bson::encode_message(&m, "Crate", &value(src)).unwrap_err();
        let p = r#"p: { name: "Cy", age: 5 }"#;
        assert!(err(&format!(r#"{{ thing: {{ name: "Al" }}, things: [], {p} }}"#)).contains("`Crate.thing` is any message, written with its type, `Name { ... }`"));
        assert!(err(&format!(r#"{{ thing: Nope {{}}, things: [], {p} }}"#)).contains("no module in the image declares a message type `Nope`"));
        assert!(err(&format!(r#"{{ thing: Person {{ name: "Al" }}, things: [], {p} }}"#)).contains("`Crate.thing` is missing field `age`"));
        assert!(err(r#"{ thing: Person { name: "Al", age: 1 }, things: [], p: Record {} }"#).contains("`Crate.p` is a `Person`, not a `Record`"));
        assert_eq!(value("Person { age: 1 }").to_string(), "Person { age: 1 }");
    }

    #[test]
    fn importing_a_type_brings_the_types_it_names() {
        let mut import = |_: &str| Ok(messages(NESTED));
        let mut parse = |src: &str| Bson::parse_messages(src, &mut import);
        let m = parse(r#"import { Record } from "./r.oth""#).unwrap();
        assert!(m.declared.is_empty() && m.types.contains_key("Person"), "{m:?}");
        assert!(parse(r#"import { Record, Person } from "./r.oth""#).is_ok(), "and may name them too");
        assert!(parse(r#"import { Record } from "./r.oth" message Mine { p: Person }"#).is_ok());
        let e = format!("{:#}", parse(r#"import { Record } from "./r.oth" message Person { name: string, age: u8 }"#).unwrap_err());
        assert!(e.contains("`Person` comes from \"./r.oth\" with a type imported from there, and is declared here too"), "{e}");
        let e = format!("{:#}", parse(r#"message Person { name: string } import { Record } from "./r.oth""#).unwrap_err());
        assert!(e.contains("`Record` needs \"./r.oth\"'s `Person`, which is not the `Person` already here"), "{e}");
    }

    #[test]
    fn literals_are_encoded_against_the_schemas() {
        let m = messages(GREETING);
        let bytes = Bson::encode_message(&m, "Greeting", &value(r#"{ excited: true, prefix: "Hi" }"#)).unwrap();
        let msg = Message::decode(&bytes).unwrap();
        assert_eq!(msg.type_name, "Greeting", "the type goes on the wire");
        assert_eq!(msg.fields.keys().collect::<Vec<_>>(), ["prefix", "excited"], "in declared order");
        let err = |ty: &str, src: &str| Bson::encode_message(&m, ty, &value(src)).unwrap_err();
        assert!(err("Greeting", r#"{ prefix: "Hi" }"#).contains("missing field `excited`"));
        assert!(err("Greeting", r#"{ $type: "Greeting", prefix: "Hi", excited: true }"#).contains("named before its braces, `Greeting { ... }`, not with `$type`"));
        assert!(err("Nope", "{}").contains("no module in the image declares a message type `Nope`"));
        assert!(err("Greeting", r#"{ prefix: 1, excited: true }"#).contains("`Greeting.prefix` is string, but was given 1"));
        assert!(err("Odd", r#"{ a: 300, b: 1, c: 1, d: "x" }"#).contains("300 does not fit in u8"));
        assert!(err("Odd", r#"{ a: 1, b: [1], c: 1, d: "x" }"#).contains("`Odd.b` is f32, but was given [1]"));
        assert!(err("Greeting", r#""text""#).contains("a message is written `Greeting { <field>: <value>, ... }`"));
        let formats = format::Formats::new(vec![Box::new(Bson)]);
        let all = format::AllSchemas::from([(BSON.to_string(), Bson::schemas(&m))]);
        assert!(formats.encode(&ContentType::from("text/plain"), "Odd", &all, &value("{}")).unwrap_err().contains("no format writes `text/plain`"));
        assert!(formats.encode(&ContentType::from("application/bson"), "Odd", &all, &value(r#"{ a: 1, b: 1, c: 1, d: "x" }"#)).is_ok());
        let schemas = Bson::schemas(&m);
        assert_eq!(Bson.fields(&schemas, "Odd").unwrap().iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d"]);
        assert_eq!(Bson.literal(&schemas, "Odd", "a", "7"), Ok(Value::Int(7)));
        assert!(Bson.literal(&schemas, "Odd", "z", "7").unwrap_err().contains("`Odd` has no field `z`"));
        assert!(Bson.check(&schemas, "Greeting", &value(r#"{ prefix: "Hi" }"#)).is_ok(), "a default may leave fields out");
    }
}
