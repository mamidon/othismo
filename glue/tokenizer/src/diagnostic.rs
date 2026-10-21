//! What the tokenizer complains about.

use crate::span::Span;

/// A lexical problem, and exactly where it is.
///
/// Spans are as tight as the problem: the offending escape, not the whole
/// string; the offending suffix, not the whole literal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(kind: DiagnosticKind, span: Span) -> Diagnostic {
        Diagnostic { kind, span }
    }

    pub fn message(&self) -> &'static str {
        self.kind.message()
    }

    pub fn severity(&self) -> Severity {
        self.kind.severity()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
}

/// Data-free, so `message` is a `&'static str` and there is no formatting on
/// the path the language server runs per keystroke. The span carries the text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiagnosticKind {
    // ---- Unterminated things ----------------------------------------------
    UnterminatedString,
    UnterminatedChar,
    UnterminatedBlockComment,

    // ---- Character literals -----------------------------------------------
    EmptyChar,
    OverlongChar,

    // ---- Escapes ----------------------------------------------------------
    UnknownEscape,
    UnicodeEscapeMissingBrace,
    UnicodeEscapeUnterminated,
    UnicodeEscapeEmpty,
    UnicodeEscapeTooLong,
    UnicodeEscapeInvalidDigit,
    UnicodeEscapeInvalidScalar,

    // ---- Numbers ----------------------------------------------------------
    MissingDigits,
    LeadingUnderscore,
    TrailingUnderscore,
    UppercaseRadixPrefix,
    UnknownSuffix,
    FloatWithIntegerSuffix,
    IntegerTooLarge,
    FloatOutOfRange,

    // ---- Stray text -------------------------------------------------------
    NonAsciiIdentifier,
    UnexpectedCharacter,
}

impl DiagnosticKind {
    pub fn message(&self) -> &'static str {
        use DiagnosticKind::*;
        match self {
            UnterminatedString => "unterminated string literal — expected a closing `\"`",
            UnterminatedChar => "unterminated character literal — expected a closing `'`",
            UnterminatedBlockComment => "unterminated block comment — expected a closing `*/`",

            EmptyChar => "empty character literal — a `char` is exactly one character",
            OverlongChar => "character literal holds more than one character",

            UnknownEscape => {
                "unknown escape sequence — expected one of `\\n` `\\r` `\\t` `\\0` `\\\\` `\\\"` `\\'` `\\u{…}`"
            }
            UnicodeEscapeMissingBrace => "expected `{` after `\\u`",
            UnicodeEscapeUnterminated => "unterminated unicode escape — expected a closing `}`",
            UnicodeEscapeEmpty => "empty unicode escape — expected at least one hex digit",
            UnicodeEscapeTooLong => "unicode escape has more than six hex digits",
            UnicodeEscapeInvalidDigit => "invalid hex digit in unicode escape",
            UnicodeEscapeInvalidScalar => {
                "not a unicode scalar value — must be at most 10FFFF and not a surrogate"
            }

            MissingDigits => "numeric literal has no digits",
            LeadingUnderscore => "`_` may separate digits, but may not lead a digit run",
            TrailingUnderscore => "`_` may separate digits, but may not trail a digit run",
            UppercaseRadixPrefix => "radix prefix must be lowercase — write `0x`, `0o`, or `0b`",
            UnknownSuffix => {
                "unknown numeric suffix — expected one of `u8` `u16` `u32` `u64` `s8` `s16` `s32` `s64` `f32` `f64`"
            }
            FloatWithIntegerSuffix => "a float literal cannot have an integer suffix",
            IntegerTooLarge => "integer literal is too large to represent",
            FloatOutOfRange => "float literal is out of range",

            NonAsciiIdentifier => "identifiers are ASCII only",
            UnexpectedCharacter => "unexpected character",
        }
    }

    pub fn severity(&self) -> Severity {
        // Everything lexical is an error today. `Severity` exists because the
        // language server wants one uniform path, and because the parser has
        // warnings of its own to report through it.
        Severity::Error
    }
}
