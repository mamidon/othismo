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
use format::{Bindings, Format, Schemas};
use messages::{FieldType, MessageType};
use protocol::payload::json_string;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;
use syntax::image_definition::{Ident, Parser, Value};
use wire::Message;

/// The content type of a BSON payload.
pub const BSON: &str = "application/bson";

/// BSON messages, as a [`Format`].
pub struct Bson;

/// A set of message types, and the ones declared where it was read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Messages {
    pub types: BTreeMap<String, MessageType>,
    /// The ones the file declares itself, in order; the rest it imports.
    pub declared: Vec<String>,
}

impl Messages {
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
        if let Some(missing) = ty.fields.iter().find(|f| !fields.contains_key(&f.name)) {
            return Err(format!("`{type_name}` is missing field `{}`", missing.name));
        }
        Ok(Message { type_name: type_name.to_string(), fields }.encode())
    }

    pub fn document_messages(schemas: &Messages, h: &str) -> String {
        let mut out = String::new();
        let types: Vec<&MessageType> = schemas.declared.iter().filter_map(|t| schemas.types.get(t)).collect();
        if types.is_empty() {
            return out;
        }
        writeln!(out, "{h} Messages\n\nPayloads of `{}`, each named by its `$type`.\n", BSON).unwrap();
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
                writeln!(out, "| `{}` | {} | {doc} |", f.name, f.ty).unwrap();
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
        Bson::messages(a).types == Bson::messages(b).types
    }

    fn encode(&self, schemas: &Schemas, ty: &str, value: &Value) -> Result<Vec<u8>, String> {
        Bson::encode_message(&Bson::messages(schemas), ty, value)
    }

    fn check(&self, schemas: &Schemas, ty: &str, value: &Value) -> Result<(), String> {
        fields(&Bson::messages(schemas), ty, value).map(|_| ())
    }

    fn fields(&self, schemas: &Schemas, ty: &str) -> Result<Vec<String>, String> {
        let messages = Bson::messages(schemas);
        Ok(message_type(&messages, ty)?.fields.iter().map(|f| f.name.clone()).collect())
    }

    fn literal(&self, schemas: &Schemas, ty: &str, field: &str, raw: &str) -> Result<Value, String> {
        let messages = Bson::messages(schemas);
        let ty = message_type(&messages, ty)?;
        match ty.field(field) {
            Some(f) => literal(&f.ty, raw),
            None => Err(format!("`{}` has no field `{field}`", ty.name)),
        }
    }

    fn show(&self, bytes: &[u8]) -> Option<Vec<(String, String)>> {
        let m = Message::decode(bytes).ok()?;
        Some(m.fields.iter().map(|(f, v)| (f.clone(), json_value(v))).collect())
    }

    fn declared(&self, schemas: &Schemas) -> Vec<String> {
        Bson::messages(schemas).declared
    }

    fn bindings(&self, schemas: &Schemas, types: &[String], library: &str) -> Option<Bindings> {
        (!types.is_empty()).then(|| bindings::assemblyscript(&Bson::messages(schemas), types, library))
    }

    fn document(&self, schemas: &Schemas, h: &str) -> String {
        Bson::document_messages(&Bson::messages(schemas), h)
    }
}

/// Message type `name`, which some module or file in the image declares.
fn message_type<'a>(schemas: &'a Messages, name: &str) -> Result<&'a MessageType, String> {
    schemas.types.get(name).ok_or_else(|| format!("no module in the image declares a message type `{name}`"))
}

/// Reads `text` as a literal for a field of type `t`.
fn literal(t: &FieldType, text: &str) -> std::result::Result<Value, String> {
    let bad = || format!("`{text}` is not a valid {t}");
    Ok(match t {
        FieldType::String | FieldType::Bytes => Value::Str(text.to_string()),
        FieldType::Bool => match text {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(bad()),
        },
        FieldType::F32 | FieldType::F64 => Value::Float(text.parse().map_err(|_| bad())?),
        FieldType::Named(_) => {
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
        B::Binary(b) => match std::str::from_utf8(&b.bytes) {
            Ok(s) => json_string(s),
            Err(_) => format!("<{} bytes>", b.bytes.len()),
        },
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

/// The BSON for `value`, if it fits field type `t`. `at` says where it is.
fn to_bson(schemas: &Messages, t: &FieldType, value: &Value, at: &str) -> std::result::Result<B, String> {
    Ok(match (t, value) {
        (FieldType::String, Value::Str(s)) => B::String(s.clone()),
        (FieldType::Bool, Value::Bool(b)) => B::Boolean(*b),
        (FieldType::F32 | FieldType::F64, Value::Float(x)) => B::Double(*x),
        (FieldType::F32 | FieldType::F64, Value::Int(i)) => B::Double(*i as f64),
        (FieldType::Bytes, Value::Str(s)) => wire::bytes(s.as_bytes().to_vec()),
        (t, Value::Int(i)) if t.int_range().is_some() => {
            let (lo, hi) = t.int_range().unwrap();
            if !(lo..=hi).contains(i) {
                return Err(format!("{i} does not fit in {t}"));
            }
            wire::int_to_bson(t, *i)
        }
        (FieldType::Named(n), Value::Object(given)) => {
            let ty = message_type(schemas, n)?;
            let doc = document(schemas, ty, given, at)?;
            if let Some(missing) = ty.fields.iter().find(|f| !doc.contains_key(&f.name)) {
                return Err(format!("`{at}` is missing field `{}`", missing.name));
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

    fn messages(src: &str) -> Messages {
        Bson::parse_messages(src, &mut |p| bail!("no {p} here")).unwrap()
    }

    fn value(src: &str) -> Value {
        let mut p = Parser::new(src, false).unwrap();
        p.value().unwrap()
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
        assert!(err("message A {} message A {}").contains("message `A` is declared twice"));
        assert!(err("object A {}").contains("expected `import` or `message`"));
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
        assert!(formats.encode("text/plain", "Odd", &all, &value("{}")).unwrap_err().contains("no format writes `text/plain`"));
        assert!(formats.encode("application/bson", "Odd", &all, &value(r#"{ a: 1, b: 1, c: 1, d: "x" }"#)).is_ok());
        let schemas = Bson::schemas(&m);
        assert_eq!(Bson.fields(&schemas, "Odd"), Ok(vec!["a".into(), "b".into(), "c".into(), "d".into()]));
        assert_eq!(Bson.literal(&schemas, "Odd", "a", "7"), Ok(Value::Int(7)));
        assert!(Bson.literal(&schemas, "Odd", "z", "7").unwrap_err().contains("`Odd` has no field `z`"));
        assert!(Bson.check(&schemas, "Greeting", &value(r#"{ prefix: "Hi" }"#)).is_ok(), "a default may leave fields out");
    }
}
