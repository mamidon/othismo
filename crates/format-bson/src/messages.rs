//! Message types: a name and flat, typed fields, as headers declare them.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    String,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    /// `bytes`; `ArrayBuffer` in AssemblyScript.
    Bytes,
    /// Another message type, by name, carried as an embedded document
    /// without a `$type`: the field's type says what it is.
    Named(String),
    /// `[T]`, a list of `T`: an array. `Array<T>` in AssemblyScript.
    List(Box<FieldType>),
}

impl FieldType {
    /// A field type as a header writes it.
    pub fn from_name(s: &str) -> Option<FieldType> {
        Some(match s {
            "string" => FieldType::String,
            "bool" => FieldType::Bool,
            "i8" => FieldType::I8,
            "i16" => FieldType::I16,
            "i32" => FieldType::I32,
            "i64" => FieldType::I64,
            "u8" => FieldType::U8,
            "u16" => FieldType::U16,
            "u32" => FieldType::U32,
            "u64" => FieldType::U64,
            "f32" => FieldType::F32,
            "f64" => FieldType::F64,
            "bytes" => FieldType::Bytes,
            _ => return None,
        })
    }

    /// The message types it names.
    pub fn names(&self) -> Vec<&str> {
        match self {
            FieldType::Named(n) => vec![n],
            FieldType::List(t) => t.names(),
            _ => vec![],
        }
    }

    /// Inclusive range of values an integer type can hold.
    pub fn int_range(&self) -> Option<(i128, i128)> {
        Some(match self {
            FieldType::I8 => (i8::MIN as i128, i8::MAX as i128),
            FieldType::I16 => (i16::MIN as i128, i16::MAX as i128),
            FieldType::I32 => (i32::MIN as i128, i32::MAX as i128),
            FieldType::I64 => (i64::MIN as i128, i64::MAX as i128),
            FieldType::U8 => (0, u8::MAX as i128),
            FieldType::U16 => (0, u16::MAX as i128),
            FieldType::U32 => (0, u32::MAX as i128),
            FieldType::U64 => (0, u64::MAX as i128),
            _ => return None,
        })
    }
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            FieldType::Named(n) => return f.write_str(n),
            FieldType::List(t) => return write!(f, "[{t}]"),
            FieldType::String => "string",
            FieldType::Bool => "bool",
            FieldType::I8 => "i8",
            FieldType::I16 => "i16",
            FieldType::I32 => "i32",
            FieldType::I64 => "i64",
            FieldType::U8 => "u8",
            FieldType::U16 => "u16",
            FieldType::U32 => "u32",
            FieldType::U64 => "u64",
            FieldType::F32 => "f32",
            FieldType::F64 => "f64",
            FieldType::Bytes => "bytes",
        };
        f.write_str(s)
    }
}

/// One of a message type's fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub ty: FieldType,
}

impl Field {
    pub fn new(name: impl Into<String>, ty: FieldType) -> Field {
        Field { name: name.into(), ty }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageType {
    pub name: String,
    /// Fields in declaration order.
    pub fields: Vec<Field>,
    /// The type's `##` documentation, Markdown; empty if it has none.
    #[serde(default)]
    pub doc: String,
    /// Each field's documentation, in the same order as `fields`.
    #[serde(default)]
    pub field_docs: Vec<String>,
}

impl MessageType {
    /// An undocumented type.
    pub fn new(name: impl Into<String>, fields: Vec<Field>) -> MessageType {
        let field_docs = vec![String::new(); fields.len()];
        MessageType { name: name.into(), fields, doc: String::new(), field_docs }
    }

    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}
