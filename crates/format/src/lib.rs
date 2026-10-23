//! Payload formats: everything Othismo does with the bytes of payloads of
//! one content type. It does all of it when a module or an image is built,
//! and when a built image's slots are given values; the runtime uses none
//! of it (see `docs/protocol.md`).
//!
//! A format is a [`Format`]. Each is found by its content type in a
//! [`Formats`] registry, which whoever builds images fills: this crate
//! knows of no format itself.

use anyhow::Result;
use protocol::payload;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use syntax::image::Value;

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
    /// Its schema files' extension, beside a module's header.
    fn extension(&self) -> &'static str;
    /// The custom section a module's schemas travel in.
    fn section(&self) -> &'static str;

    /// Schemas as a module's section carries them, checked to be ones this
    /// format wrote.
    fn load(&self, bytes: &[u8]) -> Result<Schemas>;
    /// Parses and resolves a schema file. `import` gives the schemas of a
    /// file it imports from, by the path it writes.
    fn parse(&self, src: &str, import: &mut dyn FnMut(&str) -> Result<Schemas>) -> Result<Schemas>;
    /// Adds `more`, from `origin`, to `all`. Where both have a type, they
    /// must agree. `from` says where each of `all`'s types came from.
    fn merge(&self, all: &mut Schemas, from: &mut BTreeMap<String, String>, more: &Schemas, origin: &str) -> Result<()>;
    /// Whether two sets of schemas declare the same types.
    fn same(&self, a: &Schemas, b: &Schemas) -> bool;

    /// Encodes a literal, which `schemas` must allow.
    fn encode(&self, schemas: &Schemas, value: &Value) -> Result<Vec<u8>, String>;
    /// Checks a literal that may leave fields out, such as a slot's default.
    fn check(&self, schemas: &Schemas, value: &Value) -> Result<(), String>;
    /// Every field a value must give, in order, as far as `so_far` says
    /// which type it is.
    fn fields(&self, schemas: &Schemas, so_far: &[(String, Value)]) -> Vec<String>;
    /// Raw text, as `--set` gives it, as field `field` of the value `so_far`.
    fn literal(&self, schemas: &Schemas, so_far: &[(String, Value)], field: &str, raw: &str) -> Result<Value, String>;
    /// An encoded payload's fields, each as JSON-ish text, for `--check`.
    fn show(&self, bytes: &[u8]) -> Option<Vec<(String, String)>>;

    /// AssemblyScript bindings for the types a schema file declares, or
    /// `None` if it declares none.
    fn bindings(&self, schemas: &Schemas, source: &str, library: &str) -> Option<String>;
    /// A reference for the types a schema file declares, as Markdown under
    /// headings at level `h`.
    fn document(&self, schemas: &Schemas, h: &str) -> String;
}

/// The formats an image can use, found by content type or extension.
#[derive(Clone, Default)]
pub struct Formats(Arc<Vec<Box<dyn Format>>>);

impl Formats {
    pub fn new(formats: Vec<Box<dyn Format>>) -> Formats {
        Formats(Arc::new(formats))
    }

    /// The format whose payloads are of content type `ty`.
    pub fn for_type(&self, ty: &str) -> Option<&dyn Format> {
        self.0.iter().map(|f| f.as_ref()).find(|f| payload::fits(f.content_type(), ty))
    }

    /// The format whose schema files have extension `ext`.
    pub fn for_extension(&self, ext: &str) -> Option<&dyn Format> {
        self.0.iter().map(|f| f.as_ref()).find(|f| f.extension() == ext)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Format> {
        self.0.iter().map(|f| f.as_ref())
    }

    /// Encodes `value` as a payload of content type `ty`, with `schemas` as
    /// the formats' schemas. Only a format can encode a structured literal.
    pub fn encode(&self, ty: &str, schemas: &AllSchemas, value: &Value) -> Result<Vec<u8>, String> {
        let Some(f) = self.for_type(ty) else {
            let have: Vec<&str> = self.iter().map(|f| f.content_type()).collect();
            return Err(format!("no format writes `{ty}`; a structured literal needs one, and the image has {}", list(&have)));
        };
        f.encode(schemas.get(f.content_type()).unwrap_or(&Schemas::default()), value)
    }
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
