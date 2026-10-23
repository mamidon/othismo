//! The BSON format, for `application/bson`: a payload is a BSON document
//! whose first key, `$type`, names its message type. Its schemas are
//! message types, written in a `.oth` file's `schemas application/bson`
//! block.
//! It is the format the SDK supports, and the protocol knows nothing of it.

pub mod bindings;
pub mod messages;
pub mod wire;

use anyhow::{Result, bail};
use bson::{Bson as B, Document};
use format::{Bindings, Format, Schemas};
use messages::{FieldType, MessageType};
use protocol::payload::json_string;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;
use syntax::image_definition::{Parser, Source, Tok, Value};
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
                    let Source::Path(path) = &imp.from else { bail!("{pos}: message types are imported from other `.oth` files' `{BSON}` schemas") };
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

    pub fn merge_messages(all: &mut Messages, from: &mut BTreeMap<String, String>, more: &Messages, origin: &str) -> Result<()> {
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

    pub fn encode_message(schemas: &Messages, value: &Value) -> Result<Vec<u8>, String> {
        let (type_name, fields) = fields(schemas, value)?;
        let ty = &schemas.types[&type_name];
        if let Some((missing, _)) = ty.fields.iter().find(|(n, _)| !fields.contains_key(n)) {
            return Err(format!("`{type_name}` is missing field `{missing}`"));
        }
        Ok(Message { type_name, fields }.encode())
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
            for ((name, fty), doc) in ty.fields.iter().zip(&ty.field_docs) {
                let doc = doc.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|");
                writeln!(out, "| `{name}` | {fty} | {doc} |").unwrap();
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

    fn encode(&self, schemas: &Schemas, value: &Value) -> Result<Vec<u8>, String> {
        Bson::encode_message(&Bson::messages(schemas), value)
    }

    fn check(&self, schemas: &Schemas, value: &Value) -> Result<(), String> {
        match value {
            Value::Object(fields) if !fields.iter().any(|(k, _)| k.name == wire::TYPE_KEY) => Ok(()),
            _ => fields(&Bson::messages(schemas), value).map(|_| ()),
        }
    }

    fn fields(&self, schemas: &Schemas, so_far: &[(String, Value)]) -> Vec<String> {
        let mut out = vec![wire::TYPE_KEY.to_string()];
        if let Some(ty) = type_of(so_far).and_then(|t| Bson::messages(schemas).types.get(t).cloned()) {
            out.extend(ty.fields.iter().map(|(n, _)| n.clone()));
        }
        out
    }

    fn literal(&self, schemas: &Schemas, so_far: &[(String, Value)], field: &str, raw: &str) -> Result<Value, String> {
        if field == wire::TYPE_KEY {
            return Ok(Value::Str(raw.to_string()));
        }
        let messages = Bson::messages(schemas);
        let Some(ty) = type_of(so_far).and_then(|t| messages.types.get(t)) else {
            return Err(format!("give `{}` first, so that the field's type is known", wire::TYPE_KEY));
        };
        match ty.field(field) {
            Some(t) => literal(t, raw),
            None => Err(format!("`{}` has no field `{field}`", ty.name)),
        }
    }

    fn show(&self, bytes: &[u8]) -> Option<Vec<(String, String)>> {
        let m = Message::decode(bytes).ok()?;
        let mut out = vec![(wire::TYPE_KEY.to_string(), json_value(&B::String(m.type_name.clone())))];
        out.extend(m.fields.iter().map(|(f, v)| (f.clone(), json_value(v))));
        Some(out)
    }

    fn declared(&self, schemas: &Schemas) -> Vec<String> {
        Bson::messages(schemas).declared
    }

    fn bindings(&self, schemas: &Schemas, types: &[String], library: &str) -> Option<Bindings> {
        (!types.is_empty()).then(|| bindings::assemblyscript(&Bson::messages(schemas), types, &format!("{library}/bson")))
    }

    fn document(&self, schemas: &Schemas, h: &str) -> String {
        Bson::document_messages(&Bson::messages(schemas), h)
    }
}

/// The type `$type` names among `fields`, if it does.
fn type_of(fields: &[(String, Value)]) -> Option<&str> {
    fields.iter().find_map(|(f, v)| match v {
        Value::Str(t) if f == wire::TYPE_KEY => Some(t.as_str()),
        _ => None,
    })
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
        other => json_string(&other.to_string()),
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
        assert!(err("message A { x: Strin }").contains("`Strin` is not a field type"));
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

    #[test]
    fn literals_are_encoded_against_the_schemas() {
        let m = messages(GREETING);
        let bytes = Bson::encode_message(&m, &value(r#"{ excited: true, $type: "Greeting", prefix: "Hi" }"#)).unwrap();
        let msg = Message::decode(&bytes).unwrap();
        assert_eq!(msg.type_name, "Greeting");
        assert_eq!(msg.fields.keys().collect::<Vec<_>>(), ["prefix", "excited"], "in declared order");
        let err = |src: &str| Bson::encode_message(&m, &value(src)).unwrap_err();
        assert!(err(r#"{ $type: "Greeting", prefix: "Hi" }"#).contains("missing field `excited`"));
        assert!(err(r#"{ prefix: "Hi" }"#).contains("names its type with `$type"));
        assert!(err(r#"{ $type: "Nope" }"#).contains("no module in the image declares a message type `Nope`"));
        assert!(err(r#"{ $type: "Greeting", prefix: 1, excited: true }"#).contains("`Greeting.prefix` is string, but was given 1"));
        assert!(err(r#"{ $type: "Odd", a: 300, b: 1, c: 1, d: "x" }"#).contains("300 does not fit in u8"));
        assert!(err(r#"{ $type: "Odd", a: 1, b: [1], c: 1, d: "x" }"#).contains("fields are flat"));
        assert!(err(r#""text""#).contains("is written `{ $type"));
        let formats = format::Formats::new(vec![Box::new(Bson)]);
        let all = format::AllSchemas::from([(BSON.to_string(), Bson::schemas(&m))]);
        assert!(formats.encode("text/plain", &all, &value("{}")).unwrap_err().contains("no format writes `text/plain`"));
        assert!(formats.encode("application/bson", &all, &value(r#"{ $type: "Odd", a: 1, b: 1, c: 1, d: "x" }"#)).is_ok());
        assert_eq!(Bson.fields(&Bson::schemas(&m), &[]), ["$type"]);
        let so_far = [("$type".to_string(), Value::Str("Odd".into()))];
        assert_eq!(Bson.fields(&Bson::schemas(&m), &so_far), ["$type", "a", "b", "c", "d"]);
        assert_eq!(Bson.literal(&Bson::schemas(&m), &so_far, "a", "7"), Ok(Value::Int(7)));
        assert!(Bson.literal(&Bson::schemas(&m), &[], "a", "7").unwrap_err().contains("give `$type` first"));
    }
}
