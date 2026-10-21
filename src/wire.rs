//! The message wire format shared with the AssemblyScript library.
//!
//! A message is one BSON document. Its first key is `$type`, holding the
//! message class name; the remaining keys are the class's fields. Field
//! types map to BSON as follows (mirrored in `as/index.ts`):
//!
//! | AssemblyScript              | BSON    |
//! |-----------------------------|---------|
//! | string                      | string  |
//! | bool                        | boolean |
//! | i8, i16, i32, u8, u16       | int32   |
//! | i64, u32, u64               | int64   |
//! | f32, f64                    | double  |
//! | ArrayBuffer                 | binary (generic subtype) |

use crate::messages::{FieldType, MessageType};
use anyhow::{Context, Result, bail};
use bson::{Bson, Document};
use serde::{Deserialize, Serialize};

pub const TYPE_KEY: &str = "$type";

/// A decoded message in flight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub type_name: String,
    /// The message's fields, without `$type`.
    pub fields: Document,
}

impl Message {
    pub fn decode(bytes: &[u8]) -> Result<Message> {
        let mut doc = Document::from_reader(bytes).context("malformed BSON message")?;
        let type_name = match doc.remove(TYPE_KEY) {
            Some(Bson::String(s)) => s,
            Some(other) => bail!("message `{TYPE_KEY}` must be a string, got {other}"),
            None => bail!("message has no `{TYPE_KEY}` key"),
        };
        Ok(Message { type_name, fields: doc })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut doc = Document::new();
        doc.insert(TYPE_KEY, self.type_name.clone());
        doc.extend(self.fields.clone());
        let mut out = Vec::new();
        doc.to_writer(&mut out).expect("writing BSON to a Vec cannot fail");
        out
    }

    /// Verifies the fields match `ty`'s declaration exactly.
    pub fn check_shape(&self, ty: &MessageType) -> Result<()> {
        for (name, fty) in &ty.fields {
            let Some(v) = self.fields.get(name) else {
                bail!("`{}` is missing field `{name}`", self.type_name);
            };
            let ok = match (fty, v) {
                (FieldType::String, Bson::String(_)) => true,
                (FieldType::Bool, Bson::Boolean(_)) => true,
                (FieldType::F32 | FieldType::F64, Bson::Double(_)) => true,
                (FieldType::Bytes, Bson::Binary(_)) => true,
                (t, Bson::Int32(i)) => t.int_range().is_some_and(|(lo, hi)| (lo..=hi).contains(&(*i as i128))),
                // u64 travels as the bit pattern of an int64, so any value is valid.
                (FieldType::U64, Bson::Int64(_)) => true,
                (t, Bson::Int64(i)) => t.int_range().is_some_and(|(lo, hi)| (lo..=hi).contains(&(*i as i128))),
                _ => false,
            };
            if !ok {
                bail!("`{}.{name}` should be {fty}, got {v}", self.type_name);
            }
        }
        if let Some(extra) = self.fields.keys().find(|k| ty.field(k).is_none()) {
            bail!("`{}` has unexpected field `{extra}`", self.type_name);
        }
        Ok(())
    }
}

/// The BSON value an `ArrayBuffer` field is carried as.
pub fn bytes(data: Vec<u8>) -> Bson {
    Bson::Binary(bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: data })
}

/// The BSON value an integer of type `ty` is carried as.
pub fn int_to_bson(ty: FieldType, v: i128) -> Bson {
    match ty {
        FieldType::I8 | FieldType::I16 | FieldType::I32 | FieldType::U8 | FieldType::U16 => Bson::Int32(v as i32),
        // Two's-complement bit pattern, so u64 values above i64::MAX round-trip.
        _ => Bson::Int64(v as u64 as i64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_type_first() {
        let mut fields = Document::new();
        fields.insert("text", "hi");
        let msg = Message { type_name: "Line".into(), fields };
        let bytes = msg.encode();
        let raw = Document::from_reader(&bytes[..]).unwrap();
        assert_eq!(raw.keys().next().unwrap(), TYPE_KEY);
        assert_eq!(Message::decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn shape_check() {
        let ty = MessageType { name: "P".into(), fields: vec![("x".into(), FieldType::U8)] };
        let mut fields = Document::new();
        fields.insert("x", 300);
        let msg = Message { type_name: "P".into(), fields };
        assert!(msg.check_shape(&ty).is_err());
    }
}
