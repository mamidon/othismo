//! Payload formats: everything Othismo does with the bytes of payloads of
//! one content type. It does all of it when a module or an image is built;
//! the runtime uses none of it (see `docs/protocol.md`).
//!
//! A format is a [`Format`], and [`Bson`] is the first, for
//! `application/bson`: its schemas are message types, written in `.messages`
//! files beside a module's header.

use crate::image::{Parser, Source, Tok, Value};
use crate::messages::{FieldType, MessageType};
use crate::payload;
use crate::wire::{self, Message};
use anyhow::{Result, bail};
use bson::{Bson as B, Document};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;

/// What a format supplies: a schema language, when two schemas agree, an
/// encoder for literals, bindings and documentation.
pub trait Format {
    /// A set of the format's schemas: every type a module or an image can name.
    type Schemas: Clone + Default + PartialEq + Serialize + DeserializeOwned;
    /// The content type it is for.
    const CONTENT_TYPE: &'static str;
    /// Its schema files' extension, beside a module's header.
    const EXTENSION: &'static str;
    /// The custom section a module's schemas travel in.
    const SECTION: &'static str;

    /// Parses and resolves a schema file. `import` gives the schemas of a
    /// file it imports from, by the path it writes.
    fn parse(src: &str, import: &mut dyn FnMut(&str) -> Result<Self::Schemas>) -> Result<Self::Schemas>;
    /// Adds `more`, from `origin`, to `all`. Where both have a type, they
    /// must agree. `from` says where each of `all`'s came from.
    fn merge(all: &mut Self::Schemas, from: &mut BTreeMap<String, String>, more: &Self::Schemas, origin: &str) -> Result<()>;
    /// Encodes a literal, which `schemas` must allow.
    fn encode(schemas: &Self::Schemas, value: &Value) -> Result<Vec<u8>, String>;
    /// AssemblyScript bindings for the types a schema file declares.
    fn bindings(schemas: &Self::Schemas, source: &str, library: &str) -> String;
    /// A reference for the types a schema file declares, as Markdown under
    /// headings at level `h`.
    fn document(schemas: &Self::Schemas, h: &str) -> String;
}

/// BSON messages: a document whose first key, `$type`, names its type.
pub struct Bson;

/// A set of message types, and the ones declared where it was read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Messages {
    pub types: BTreeMap<String, MessageType>,
    /// The ones the file declares itself, in order; the rest it imports.
    pub declared: Vec<String>,
}

impl Format for Bson {
    type Schemas = Messages;
    const CONTENT_TYPE: &'static str = payload::BSON;
    const EXTENSION: &'static str = "messages";
    const SECTION: &'static str = "othismo.schema.bson";

    fn parse(src: &str, import: &mut dyn FnMut(&str) -> Result<Messages>) -> Result<Messages> {
        let mut p = Parser::new(src, true)?;
        let mut out = Messages::default();
        let mut from: BTreeMap<String, String> = BTreeMap::new();
        loop {
            let doc = p.docs();
            let pos = p.pos();
            match p.peek().clone() {
                Tok::Eof => {
                    p.no_docs(&doc, pos)?;
                    return Ok(out);
                }
                Tok::Ident(kw) if kw == "import" => {
                    p.no_docs(&doc, pos)?;
                    let imp = p.import()?;
                    let Source::Path(path) = &imp.from else { bail!("{pos}: message types are imported from other `.messages` files") };
                    let theirs = import(path).map_err(|e| anyhow::anyhow!("{pos}: importing from {path:?}: {e:#}"))?;
                    for n in &imp.names {
                        if n.local.name != n.name.name {
                            bail!("{}: a message type is named on the wire, so `{}` can't be renamed", n.local.pos, n.name.name);
                        }
                        let Some(ty) = theirs.types.get(&n.name.name) else { bail!("{}: {path:?} has no message type `{}`", n.name.pos, n.name.name) };
                        if from.insert(n.name.name.clone(), path.clone()).is_some() || out.types.contains_key(&n.name.name) {
                            bail!("{}: `{}` is already imported", n.name.pos, n.name.name);
                        }
                        out.types.insert(n.name.name.clone(), ty.clone());
                    }
                }
                Tok::Ident(kw) if kw == "message" => {
                    let m = p.message(doc)?;
                    let name = &m.name;
                    if let Some(prev) = from.get(&name.name) {
                        bail!("{}: `{}` is imported from {prev:?}, and declared here too", name.pos, name.name);
                    }
                    if out.types.contains_key(&name.name) {
                        bail!("{}: message `{}` is declared twice", name.pos, name.name);
                    }
                    let mut fields: Vec<(String, FieldType)> = vec![];
                    for f in &m.fields {
                        if fields.iter().any(|(n, _)| *n == f.name.name) {
                            bail!("{}: `{}` has two fields called `{}`", f.name.pos, name.name, f.name.name);
                        }
                        let Some(ty) = FieldType::from_name(&f.ty.name) else {
                            bail!(
                                "{}: `{}` is not a field type; fields are string, bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64 or bytes",
                                f.ty.pos,
                                f.ty.name
                            );
                        };
                        fields.push((f.name.name.clone(), ty));
                    }
                    let field_docs = m.fields.iter().map(|f| f.doc.clone()).collect();
                    let ty = MessageType { name: name.name.clone(), fields, doc: m.doc.clone(), field_docs };
                    out.types.insert(name.name.clone(), ty);
                    out.declared.push(name.name.clone());
                }
                other => bail!("{pos}: expected `import` or `message`, found {other}"),
            }
        }
    }

    fn merge(all: &mut Messages, from: &mut BTreeMap<String, String>, more: &Messages, origin: &str) -> Result<()> {
        for (name, ty) in &more.types {
            match all.types.get(name) {
                Some(prev) if prev.fields != ty.fields => {
                    bail!("`{name}` is declared one way by {} and another by {origin}; an image can't use both", from[name])
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

    fn encode(schemas: &Messages, value: &Value) -> Result<Vec<u8>, String> {
        let (type_name, fields) = fields(schemas, value)?;
        let ty = &schemas.types[&type_name];
        if let Some((missing, _)) = ty.fields.iter().find(|(n, _)| !fields.contains_key(n)) {
            return Err(format!("`{type_name}` is missing field `{missing}`"));
        }
        Ok(Message { type_name, fields }.encode())
    }

    fn bindings(schemas: &Messages, source: &str, library: &str) -> String {
        crate::bindings::assemblyscript(schemas, source, library)
    }

    fn document(schemas: &Messages, h: &str) -> String {
        let mut out = String::new();
        let types: Vec<&MessageType> = schemas.declared.iter().filter_map(|t| schemas.types.get(t)).collect();
        if types.is_empty() {
            return out;
        }
        writeln!(out, "{h} Messages\n\nPayloads of `{}`, each named by its `$type`.\n", Bson::CONTENT_TYPE).unwrap();
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
            for ((name, fty), doc) in ty.fields.iter().zip(&ty.field_docs) {
                let doc = doc.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|");
                writeln!(out, "| `{name}` | {fty} | {doc} |").unwrap();
            }
            out.push('\n');
        }
        out
    }
}

/// The type a BSON literal names with `$type`, and its fields as given,
/// each checked against the type. Fields left out are fine here.
pub fn fields(schemas: &Messages, value: &Value) -> Result<(String, Document), String> {
    let Value::Object(given) = value else { return Err("a BSON payload is written `{ $type: \"<type>\", <field>: <value>, ... }`".into()) };
    let Some((_, Value::Str(type_name))) = given.iter().find(|(k, _)| k.name == wire::TYPE_KEY) else {
        return Err(format!("a BSON payload names its type with `{}: \"<type>\"`", wire::TYPE_KEY));
    };
    let Some(ty) = schemas.types.get(type_name) else {
        return Err(format!("no module in the image declares a message type `{type_name}`"));
    };
    let mut fields = Document::new();
    for (name, v) in given.iter().filter(|(k, _)| k.name != wire::TYPE_KEY) {
        if fields.contains_key(&name.name) {
            return Err(format!("field `{}` given twice", name.name));
        }
        fields.insert(name.name.clone(), field_bson(ty, &name.name, v)?);
    }
    // In declared order, as the module's own encoder writes them.
    let mut ordered = Document::new();
    for (n, _) in &ty.fields {
        if let Some(v) = fields.get(n) {
            ordered.insert(n.clone(), v.clone());
        }
    }
    Ok((type_name.clone(), ordered))
}

/// The BSON for `value` as field `field` of `ty`, if it fits the field's type.
pub fn field_bson(ty: &MessageType, field: &str, value: &Value) -> std::result::Result<B, String> {
    let Some(fty) = ty.field(field) else {
        return Err(format!("`{}` has no field `{field}`", ty.name));
    };
    Ok(match (fty, value) {
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
        (_, Value::Object(_) | Value::List(_)) => {
            return Err(format!("`{}.{field}` is {fty}; a BSON message's fields are flat, so far", ty.name));
        }
        (t, v) => return Err(format!("`{}.{field}` is {t}, but was given {v}", ty.name)),
    })
}

/// Encodes `value` as a payload of content type `ty`, with `messages` as
/// the BSON format's schemas. Only a format can encode a structured literal.
pub fn encode(ty: &str, messages: &Messages, value: &Value) -> Result<Vec<u8>, String> {
    if payload::fits(Bson::CONTENT_TYPE, ty) {
        return Bson::encode(messages, value);
    }
    Err(format!("no format writes `{ty}`; a structured literal needs one, and the runtime has {}", Bson::CONTENT_TYPE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(src: &str) -> Messages {
        Bson::parse(src, &mut |p| bail!("no {p} here")).unwrap()
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
        let err = |src: &str| format!("{:#}", Bson::parse(src, &mut |p| bail!("no {p} here")).unwrap_err());
        assert!(err("message A { x: Strin }").contains("`Strin` is not a field type"));
        assert!(err("message A { x: i32 x: i32 }").contains("two fields called `x`"));
        assert!(err("message A {} message A {}").contains("message `A` is declared twice"));
        assert!(err("object A {}").contains("expected `import` or `message`"));
        assert!(err(r#"import { A } from "./a.messages""#).contains("no ./a.messages here"));

        let mut import = |_: &str| Ok(messages(GREETING));
        let m = Bson::parse(r#"import { Greeting } from "./g.messages" message Mine { g: string }"#, &mut import).unwrap();
        assert_eq!(m.declared, ["Mine"], "imported types have their own bindings");
        assert!(m.types.contains_key("Greeting"));
        let e = format!("{:#}", Bson::parse(r#"import { Greeting as G } from "./g.messages""#, &mut import).unwrap_err());
        assert!(e.contains("can't be renamed"), "{e}");
    }

    #[test]
    fn literals_are_encoded_against_the_schemas() {
        let m = messages(GREETING);
        let bytes = Bson::encode(&m, &value(r#"{ excited: true, $type: "Greeting", prefix: "Hi" }"#)).unwrap();
        let msg = Message::decode(&bytes).unwrap();
        assert_eq!(msg.type_name, "Greeting");
        assert_eq!(msg.fields.keys().collect::<Vec<_>>(), ["prefix", "excited"], "in declared order");
        let err = |src: &str| Bson::encode(&m, &value(src)).unwrap_err();
        assert!(err(r#"{ $type: "Greeting", prefix: "Hi" }"#).contains("missing field `excited`"));
        assert!(err(r#"{ prefix: "Hi" }"#).contains("names its type with `$type"));
        assert!(err(r#"{ $type: "Nope" }"#).contains("no module in the image declares a message type `Nope`"));
        assert!(err(r#"{ $type: "Greeting", prefix: 1, excited: true }"#).contains("`Greeting.prefix` is string, but was given 1"));
        assert!(err(r#"{ $type: "Odd", a: 300, b: 1, c: 1, d: "x" }"#).contains("300 does not fit in u8"));
        assert!(err(r#"{ $type: "Odd", a: 1, b: [1], c: 1, d: "x" }"#).contains("fields are flat"));
        assert!(err(r#""text""#).contains("is written `{ $type"));
        assert!(encode("text/plain", &m, &value("{}")).unwrap_err().contains("no format writes `text/plain`"));
    }
}
