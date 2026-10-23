//! The BSON format's wire layout, shared with the AssemblyScript library.
//!
//! A message is one BSON document. Its first key is `$type`, holding the
//! message type's name; the remaining keys are its fields. Field types map
//! to BSON as follows (mirrored in `sdk/assemblyscript/bson.ts`):
//!
//! | Header                      | BSON    |
//! |-----------------------------|---------|
//! | string                      | string  |
//! | bool                        | boolean |
//! | i8, i16, i32, u8, u16       | int32   |
//! | i64, u32, u64               | int64   |
//! | f32, f64                    | double  |
//! | bytes                       | binary (generic subtype) |
//! | time                        | UTC datetime |
//! | uuid                        | binary (UUID subtype) |
//! | a message type, `Person`    | document, without `$type` |
//! | `[T]`, a list               | array |
//! | `{T}`, a map                | document |
//!
//! An optional field, `name?: T`, is left out when it has no value; a
//! reader takes a null as left out too.

use crate::messages::FieldType;
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
}

/// The BSON value a `bytes` field is carried as.
pub fn bytes(data: Vec<u8>) -> Bson {
    Bson::Binary(bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: data })
}

/// The BSON value a `uuid` field is carried as.
pub fn uuid(id: [u8; 16]) -> Bson {
    Bson::Binary(bson::Binary { subtype: bson::spec::BinarySubtype::Uuid, bytes: id.to_vec() })
}

/// A UUID written as people do, `123e4567-e89b-12d3-a456-426614174000`.
pub fn parse_uuid(s: &str) -> Option<[u8; 16]> {
    let groups: Vec<&str> = s.split('-').collect();
    if groups.iter().map(|g| g.len()).collect::<Vec<_>>() != [8, 4, 4, 4, 12] {
        return None;
    }
    let hex = groups.concat();
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// `id` as people write it.
pub fn show_uuid(id: &[u8]) -> String {
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// The BSON value an integer of type `ty` is carried as.
pub fn int_to_bson(ty: &FieldType, v: i128) -> Bson {
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
    fn uuids_are_written_as_people_do() {
        let id = parse_uuid("123E4567-e89b-12d3-a456-426614174000").unwrap();
        assert_eq!(id[0], 0x12);
        assert_eq!(show_uuid(&id), "123e4567-e89b-12d3-a456-426614174000");
        for bad in ["", "123e4567e89b12d3a456426614174000", "123e4567-e89b-12d3-a456-42661417400g", "123e4567-e89b-12d3-a456-4266141740000"] {
            assert_eq!(parse_uuid(bad), None, "{bad}");
        }
    }

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
}
