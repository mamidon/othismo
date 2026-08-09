//! Decoding a literal token into the value it names.
//!
//! Deliberately separate from lexing. A `Token` is a kind and a span, so this
//! is the seam where a span becomes a `u128`, an `f64`, or a `Cow<str>` — and
//! the parser only pays for it on the literals it actually builds nodes for.

use std::borrow::Cow;

use crate::cursor::Cursor;
use crate::escape::consume_escape;
use crate::number::{NumberValue, consume_number};
use crate::token::{Token, TokenKind};

/// The value a literal token names.
///
/// An integer arrives as a `u128` and a float as an `f64`, with the suffix, if
/// any, alongside. Neither is the literal's *type*: §1 makes an unsuffixed
/// literal an unpinned constant that acquires a type from context, and pinning
/// is the type checker's job, not the tokenizer's.
#[derive(Clone, PartialEq, Debug)]
pub enum Literal<'src> {
    Int {
        value: u128,
        suffix: Option<NumericType>,
    },
    Float {
        value: f64,
        suffix: Option<NumericType>,
    },
    /// Borrows the source when the literal holds no escapes, which is nearly
    /// always.
    Str(Cow<'src, str>),
    Char(char),
    Bool(bool),
}

/// The types a numeric suffix can name (§1). `s` rather than `i` for signed,
/// matching wasm's own `s`/`u` instruction suffixes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NumericType {
    U8,
    U16,
    U32,
    U64,
    S8,
    S16,
    S32,
    S64,
    F32,
    F64,
}

impl NumericType {
    pub fn from_name(name: &str) -> Option<NumericType> {
        use NumericType::*;
        Some(match name {
            "u8" => U8,
            "u16" => U16,
            "u32" => U32,
            "u64" => U64,
            "s8" => S8,
            "s16" => S16,
            "s32" => S32,
            "s64" => S64,
            "f32" => F32,
            "f64" => F64,
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        use NumericType::*;
        match self {
            U8 => "u8",
            U16 => "u16",
            U32 => "u32",
            U64 => "u64",
            S8 => "s8",
            S16 => "s16",
            S32 => "s32",
            S64 => "s64",
            F32 => "f32",
            F64 => "f64",
        }
    }

    pub fn is_integer(&self) -> bool {
        !matches!(self, NumericType::F32 | NumericType::F64)
    }

    pub fn is_signed(&self) -> bool {
        use NumericType::*;
        matches!(self, S8 | S16 | S32 | S64)
    }
}

/// Decode a literal token's value.
///
/// `None` when the token isn't a literal, or when it was malformed past
/// recovery — in which case `tokenize` already reported a diagnostic saying
/// why, and there is nothing to add. A string with one bad escape still
/// decodes: the bad escape contributes nothing and the rest of the text
/// survives, because half a string is more use to the editor than none.
pub fn literal_value<'src>(token: Token, source: &'src str) -> Option<Literal<'src>> {
    let text = token.text(source);
    match token.kind {
        TokenKind::True => Some(Literal::Bool(true)),
        TokenKind::False => Some(Literal::Bool(false)),

        TokenKind::Int | TokenKind::Float => {
            let scan = consume_number(&mut Cursor::new(text.as_bytes()), text);
            match scan.value? {
                NumberValue::Int(value) => Some(Literal::Int {
                    value,
                    suffix: scan.suffix,
                }),
                NumberValue::Float(value) => Some(Literal::Float {
                    value,
                    suffix: scan.suffix,
                }),
            }
        }

        TokenKind::Str => Some(Literal::Str(decode(body(text, 1, "\"")))),
        TokenKind::Char => decode_char(body(text, 1, "'")).map(Literal::Char),

        _ => None,
    }
}

/// The text between the delimiters.
///
/// An unterminated literal has no closing delimiter — the lexer stops it at the
/// newline rather than swallowing the file — so the closing one is only removed
/// if it's actually there.
fn body<'src>(text: &'src str, open: usize, close: &str) -> &'src str {
    let inner = &text[open.min(text.len())..];
    match inner.strip_suffix(close) {
        // Guard against a literal that is *only* its opening delimiter, where
        // the opener would otherwise be mistaken for the closer.
        Some(stripped) if text.len() >= open + close.len() => stripped,
        _ => inner,
    }
}

/// Process escapes.
///
/// §1 also asks for CRLF to be normalized to LF, and there is nowhere left for
/// that to happen: a string stops at a newline, so no literal can contain a
/// raw one. Normalization comes back with whatever multi-line form does.
fn decode(inner: &str) -> Cow<'_, str> {
    if !inner.contains('\\') {
        return Cow::Borrowed(inner);
    }

    let mut decoded = String::with_capacity(inner.len());
    let mut cursor = Cursor::new(inner.as_bytes());
    while let Some(byte) = cursor.peek(0) {
        match byte {
            b'\\' => {
                if let Ok(character) = consume_escape(&mut cursor) {
                    decoded.push(character);
                }
            }
            _ => {
                let start = cursor.index();
                cursor.consume_utf8();
                decoded.push_str(&inner[start..cursor.index()]);
            }
        }
    }
    Cow::Owned(decoded)
}

fn decode_char(inner: &str) -> Option<char> {
    let mut cursor = Cursor::new(inner.as_bytes());
    let character = match cursor.peek(0)? {
        b'\\' => consume_escape(&mut cursor).ok()?,
        _ => {
            let start = cursor.index();
            cursor.consume_utf8();
            inner[start..cursor.index()].chars().next()?
        }
    };
    // Exactly one character, or it isn't a `char` (§1).
    cursor.at_end().then_some(character)
}
