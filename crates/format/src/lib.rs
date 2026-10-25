//! Payload formats: everything Othismo does with the bytes of payloads of
//! one content type. It does all of it when a module or an image is built,
//! and when a built image's slots are given values; the runtime uses none
//! of it, beyond [`json_string`] as it prints (see `docs/protocol.md`).
//!
//! A format is a [`Format`]. Each is found by its content type in a
//! [`Formats`] registry, which whoever builds images fills: this crate
//! knows of no format itself.

use anyhow::Result;
use protocol::content_type::ContentType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;
use syntax::image_definition::Value;

/// A format's schemas, as only the format can read them: every type a
/// module or an image can name. Everything else carries them as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schemas(#[serde(with = "bytes")] pub Vec<u8>);

/// Schemas by the content type of the format they are for.
pub type AllSchemas = BTreeMap<String, Schemas>;

/// What a format supplies: a schema language, when two schemas agree, an
/// encoder for literals and for slots filled field by field, bindings and
/// documentation.
pub trait Format: Send + Sync {
    /// The content type it is for.
    fn content_type(&self) -> &'static str;
    /// The custom section a module's schemas travel in.
    fn section(&self) -> &'static str;

    /// Schemas as a module's section carries them, checked to be ones this
    /// format wrote.
    fn load(&self, bytes: &[u8]) -> Result<Schemas>;
    /// Parses and resolves the body of a `.oth` file's schemas block for
    /// its content type. `import` gives the schemas of another file's
    /// block, by the path the body writes.
    fn parse(&self, src: &str, import: &mut dyn FnMut(&str) -> Result<Schemas>) -> Result<Schemas>;
    /// Adds `more`, from `origin`, to `all`. Where both have a type, they
    /// must agree. `from` says where each of `all`'s types came from.
    fn merge(&self, all: &mut Schemas, from: &mut BTreeMap<String, String>, more: &Schemas, origin: &str) -> Result<()>;
    /// Whether two sets of schemas declare the same types.
    fn same(&self, a: &Schemas, b: &Schemas) -> bool;

    /// Encodes a literal of type `ty`, which `schemas` must have.
    fn encode(&self, schemas: &Schemas, ty: &str, value: &Value) -> Result<Vec<u8>, String>;
    /// Checks a literal of type `ty` that may leave fields out, such as a
    /// slot's default.
    fn check(&self, schemas: &Schemas, ty: &str, value: &Value) -> Result<(), String>;
    /// Every field a value of type `ty` can give, in order, and whether it
    /// must.
    fn fields(&self, schemas: &Schemas, ty: &str) -> Result<Vec<(String, bool)>, String>;
    /// Raw text, as `--set` gives it, as field `field` of a value of type `ty`.
    fn literal(&self, schemas: &Schemas, ty: &str, field: &str, raw: &str) -> Result<Value, String>;
    /// An encoded payload's fields, each as JSON-ish text, for `--check`.
    fn show(&self, bytes: &[u8]) -> Option<Vec<(String, String)>>;
    /// Message type `ty`, which `schemas` must have, as a person reads it:
    /// its documentation, and each field's, with its type. For the shell's
    /// completion.
    fn describe(&self, schemas: &Schemas, ty: &str) -> Result<MessageDoc, String>;

    /// The types `schemas` declares where they were read, in order: not
    /// those it imports.
    fn declared(&self, schemas: &Schemas) -> Vec<String>;
    /// AssemblyScript bindings for `types`, which `schemas` has, or `None`
    /// for none. `library` is the SDK, `"othismo"`, which the format's own
    /// library is found under. Each type is a class with
    /// `static fromPayload(p: Payload | null): T | null`, the value in a
    /// payload if it holds one of that type, and `toPayload(): Payload`,
    /// which the generated server bases and clients use.
    fn bindings(&self, schemas: &Schemas, types: &[String], library: &str) -> Option<Bindings>;
    /// A reference for the types a block declares, as Markdown under
    /// headings at level `h`.
    fn document(&self, schemas: &Schemas, h: &str) -> String;
}

/// A message type, described: see [`Format::describe`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageDoc {
    pub name: String,
    /// Markdown; empty if it has none.
    pub doc: String,
    pub fields: Vec<FieldDoc>,
}

/// One field of a message type: its name, its type as a schema writes it,
/// a literal of that type to start from, and its documentation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldDoc {
    pub name: String,
    pub ty: String,
    /// What the field could be given, as written in a literal: `true` and
    /// `false` for a boolean, `""` for text, `0` for a number.
    pub examples: Vec<String>,
    pub doc: String,
}

/// Generated AssemblyScript: what it imports, which goes at the top of the
/// file, and the code itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings {
    pub imports: String,
    pub code: String,
}

/// The formats an image can use, found by content type.
#[derive(Clone, Default)]
pub struct Formats(Arc<Vec<Box<dyn Format>>>);

impl Formats {
    pub fn new(formats: Vec<Box<dyn Format>>) -> Formats {
        Formats(Arc::new(formats))
    }

    /// The format whose payloads are of content type `ty`.
    pub fn for_type(&self, ty: &ContentType) -> Option<&dyn Format> {
        self.0.iter().map(|f| f.as_ref()).find(|f| ty.fits(&ContentType::from_static(f.content_type())))
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Format> {
        self.0.iter().map(|f| f.as_ref())
    }

    /// Encodes `value`, of message type `ty`, as a payload of content type
    /// `content_type`, with `schemas` as the formats' schemas. Only a format
    /// can encode a structured literal.
    pub fn encode(&self, content_type: &ContentType, ty: &str, schemas: &AllSchemas, value: &Value) -> Result<Vec<u8>, String> {
        let Some(f) = self.for_type(content_type) else {
            let have: Vec<&str> = self.iter().map(|f| f.content_type()).collect();
            return Err(format!("no format writes `{content_type}`; a structured literal needs one, and the image has {}", list(&have)));
        };
        f.encode(schemas.get(f.content_type()).unwrap_or(&Schemas::default()), ty, value)
    }
}

/// `text` as a JSON string.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn list(names: &[&str]) -> String {
    if names.is_empty() { "none".into() } else { names.join(", ") }
}

/// Serializes schemas' bytes as one binary value, not a list of numbers.
mod bytes {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(b)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        struct Bytes;
        impl<'de> serde::de::Visitor<'de> for Bytes {
            type Value = Vec<u8>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bytes")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
                Ok(v.to_vec())
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(v)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
                let mut out = vec![];
                while let Some(b) = seq.next_element::<u8>()? {
                    out.push(b);
                }
                Ok(out)
            }
        }
        d.deserialize_byte_buf(Bytes)
    }
}
