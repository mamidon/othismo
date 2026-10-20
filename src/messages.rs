//! A deliberately small reader for AssemblyScript message declarations.
//!
//! It understands just enough TypeScript to pull `export class Name { field: type }`
//! out of a file: classes, their instance fields and those fields' types.
//! Methods, constructors, static members and top-level statements are skipped.

use anyhow::{Result, bail};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// `ArrayBuffer` in AssemblyScript.
    Bytes,
}

impl FieldType {
    fn from_name(s: &str) -> Option<FieldType> {
        Some(match s {
            "string" => FieldType::String,
            "bool" | "boolean" => FieldType::Bool,
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
            "ArrayBuffer" => FieldType::Bytes,
            _ => return None,
        })
    }

    /// Inclusive range of values an integer type can hold.
    pub fn int_range(self) -> Option<(i128, i128)> {
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
            FieldType::Bytes => "ArrayBuffer",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageType {
    pub name: String,
    /// Fields in declaration order.
    pub fields: Vec<(String, FieldType)>,
}

impl MessageType {
    pub fn field(&self, name: &str) -> Option<FieldType> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, t)| *t)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Punct(char),
    Newline,
}

/// Tokenizes TypeScript loosely: identifiers, single punctuation characters
/// and newlines. Comments and string/template literals are dropped.
fn lex(src: &str) -> Vec<(Tok, usize)> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            toks.push((Tok::Newline, line));
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                if chars[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
        } else if c == '"' || c == '\'' || c == '`' {
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' {
                    i += 1;
                } else if chars[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 1;
            toks.push((Tok::Word("<literal>".into()), line));
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            toks.push((Tok::Word(chars[start..i].iter().collect()), line));
        } else {
            toks.push((Tok::Punct(c), line));
            i += 1;
        }
    }
    toks
}

const MODIFIERS: &[&str] = &["public", "private", "protected", "readonly", "declare"];

/// Extracts every exported class and its instance fields.
pub fn parse(src: &str) -> Result<BTreeMap<String, MessageType>> {
    let toks = lex(src);
    let word = |i: usize| match toks.get(i) {
        Some((Tok::Word(w), _)) => Some(w.as_str()),
        _ => None,
    };
    let punct = |i: usize, p: char| matches!(toks.get(i), Some((Tok::Punct(c), _)) if *c == p);

    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < toks.len() {
        if !(word(i) == Some("export") && word(i + 1) == Some("class")) {
            i += 1;
            continue;
        }
        let Some(name) = word(i + 2).map(str::to_string) else {
            bail!("line {}: expected class name after `export class`", toks[i].1);
        };
        // Skip `extends X implements Y` etc. up to the body.
        i += 3;
        while i < toks.len() && !punct(i, '{') {
            i += 1;
        }
        i += 1;

        let mut fields = Vec::new();
        let mut depth = 1; // brace depth relative to the class
        let mut parens = 0;
        let mut at_member_start = true;
        while i < toks.len() && depth > 0 {
            if depth == 1 && parens == 0 && at_member_start {
                let mut j = i;
                while toks.get(j).is_some_and(|t| t.0 == Tok::Newline) {
                    j += 1;
                }
                while word(j).is_some_and(|w| MODIFIERS.contains(&w)) {
                    j += 1;
                }
                // `name: type` or `name!: type`; `static` members aren't message fields.
                let is_field = word(j) != Some("static") && (punct(j + 1, ':') || (punct(j + 1, '!') && punct(j + 2, ':')));
                if let (Some(field), true) = (word(j), is_field) {
                    let ty_at = if punct(j + 1, '!') { j + 3 } else { j + 2 };
                    let line = toks.get(ty_at).map_or(0, |t| t.1);
                    let ty_name = word(ty_at).unwrap_or("?");
                    let complex = toks.get(ty_at + 1).is_some_and(|t| {
                        matches!(t.0, Tok::Punct(c) if c == '|' || c == '<' || c == '[')
                    });
                    let ty = match FieldType::from_name(ty_name) {
                        Some(ty) if !complex => ty,
                        _ => bail!(
                            "line {line}: field `{name}.{field}` has unsupported type `{ty_name}`; \
                             messages may only use string, bool, integer, float and ArrayBuffer fields"
                        ),
                    };
                    fields.push((field.to_string(), ty));
                    i = ty_at + 1;
                    at_member_start = false;
                    continue;
                }
                at_member_start = false;
            }
            match &toks[i].0 {
                Tok::Punct('{') => depth += 1,
                Tok::Punct('}') => {
                    depth -= 1;
                    at_member_start = depth == 1;
                }
                Tok::Punct('(') => parens += 1,
                Tok::Punct(')') => parens -= 1,
                Tok::Punct(';') | Tok::Newline if depth == 1 && parens == 0 => at_member_start = true,
                _ => {}
            }
            i += 1;
        }
        if out.contains_key(&name) {
            bail!("class `{name}` is declared twice");
        }
        out.insert(name.clone(), MessageType { name, fields });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_fields_and_skips_methods() {
        let src = r#"
            import { Message, Writer } from "real-talk"

            // A line of text.
            export class Line extends Message {
              text: string
              constructor(text: string) {
                super()
                this.text = text
              }
              typeName(): string { return "Line" }
              encode(w: Writer): void { w.string("text", this.text) }
            }

            export class Point {
              public x: i32 = 0;
              readonly y!: f64;
              static ORIGIN: i32 = 3
              get norm(): f64 { return 0 }
              ok: bool
              raw: ArrayBuffer
            }

            class Hidden { a: i32 }
            registerMessage("Line", (r: Reader): Message => new Line(r.string("text")))
        "#;
        let msgs = parse(src).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs["Line"].fields, vec![("text".into(), FieldType::String)]);
        assert_eq!(
            msgs["Point"].fields,
            vec![
                ("x".into(), FieldType::I32),
                ("y".into(), FieldType::F64),
                ("ok".into(), FieldType::Bool),
                ("raw".into(), FieldType::Bytes),
            ]
        );
    }

    #[test]
    fn rejects_unsupported_field_types() {
        let err = parse("export class A { xs: Array<i32> }").unwrap_err();
        assert!(err.to_string().contains("A.xs"), "{err}");
        assert!(parse("export class A { s: string | null }").is_err());
    }
}
