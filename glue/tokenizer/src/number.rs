//! Numeric literals, understood once.
//!
//! Same bargain as [`crate::escape`]: the lexer drives this to size a token and
//! report diagnostics, and [`crate::literal_value`] drives it again to produce
//! the value. One implementation of "where does the suffix end", because the
//! answer is not obvious — `0x1f32` has no suffix, since `f`, `3`, and `2` are
//! all hex digits, while `0x1u8` does — and two implementations of it would
//! eventually disagree.
//!
//! Offsets in the result are indices into whatever slice the cursor was built
//! on. The lexer builds one over the whole file and gets absolute spans; the
//! decoder builds one over a single token's text and gets offsets within it.
//! Neither has to shift anything.

use std::ops::Range;

use crate::cursor::Cursor;
use crate::diagnostic::DiagnosticKind;
use crate::literal::NumericType;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum NumberValue {
    Int(u128),
    Float(f64),
}

pub(crate) struct NumberScan {
    /// A `.` or an exponent is what makes it a float (§1).
    pub is_float: bool,
    /// `None` when the literal is malformed, or the suffix is unrecognized.
    pub suffix: Option<NumericType>,
    /// `None` when there were no digits to parse, or the value doesn't fit.
    pub value: Option<NumberValue>,
    pub errors: Vec<(Range<usize>, DiagnosticKind)>,
}

/// Consume one numeric literal. The cursor must sit on a digit, or on the `.`
/// of a leading-dot float — the caller has already applied §1's left-context
/// rule to decide which.
pub(crate) fn consume_number(cursor: &mut Cursor<u8>, source: &str) -> NumberScan {
    let start = cursor.index();
    let mut errors = Vec::new();
    let mut is_float = false;
    let mut radix = 10;

    // Where the digits proper begin — past a `0x`/`0o`/`0b` prefix, if there is
    // one, since `from_str_radix` doesn't want to see it.
    let mut digits_start = start;

    if cursor.peek(0) == Some(b'.') {
        // `.5` — a float with no integer part (§1).
        is_float = true;
        cursor.consume();
        consume_digit_run(cursor, 10, &mut errors);
        consume_exponent(cursor, &mut is_float, &mut errors);
    } else if let Some(marker) = radix_marker(cursor) {
        radix = match marker.to_ascii_lowercase() {
            b'x' => 16,
            b'o' => 8,
            _ => 2,
        };
        if marker.is_ascii_uppercase() {
            errors.push((
                cursor.index() + 1..cursor.index() + 2,
                DiagnosticKind::UppercaseRadixPrefix,
            ));
        }
        cursor.consume();
        cursor.consume();
        digits_start = cursor.index();
        if consume_digit_run(cursor, radix, &mut errors) == 0 {
            errors.push((start..cursor.index(), DiagnosticKind::MissingDigits));
        }
        // Hex, octal, and binary are integer-only (§1). A `.` after one is
        // field access, and the caller will lex it as such.
    } else {
        consume_digit_run(cursor, 10, &mut errors);

        // A *trailing* `.` is not part of a literal: `1.` is `1` then `.`, so
        // that `1.method()` needs no lookahead (§1).
        if cursor.peek(0) == Some(b'.') && cursor.peek(1).is_some_and(|b| b.is_ascii_digit()) {
            is_float = true;
            cursor.consume();
            consume_digit_run(cursor, 10, &mut errors);
        }

        consume_exponent(cursor, &mut is_float, &mut errors);
    }

    let body = start..cursor.index();

    // A suffix is an identifier run glued to the literal. Taking it as part of
    // the token even when it's nonsense — `1blah` — recovers better than
    // emitting an integer followed by an identifier that can't be there.
    let suffix_start = cursor.index();
    cursor.consume_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    let suffix_range = suffix_start..cursor.index();
    let suffix = match &source[suffix_range.clone()] {
        "" => None,
        text => match NumericType::from_name(text) {
            Some(numeric_type) => {
                if is_float && numeric_type.is_integer() {
                    errors.push((suffix_range, DiagnosticKind::FloatWithIntegerSuffix));
                }
                Some(numeric_type)
            }
            None => {
                errors.push((suffix_range, DiagnosticKind::UnknownSuffix));
                None
            }
        },
    };

    let value = if is_float {
        parse_float(&source[body.clone()], &body, &mut errors)
    } else {
        parse_int(&source[digits_start..body.end], radix, &body, &mut errors)
    };

    NumberScan {
        is_float,
        suffix,
        value,
        errors,
    }
}

/// `0x`, `0o`, `0b` — and their uppercase spellings, which are diagnosed rather
/// than left to degrade into a nonsensical "unknown suffix".
fn radix_marker(cursor: &Cursor<u8>) -> Option<u8> {
    if cursor.peek(0) != Some(b'0') {
        return None;
    }
    match cursor.peek(1) {
        Some(marker @ (b'x' | b'X' | b'o' | b'O' | b'b' | b'B')) => Some(marker),
        _ => None,
    }
}

/// Consume digits and separators, and return how many *digits* there were.
///
/// `_` may separate digits; it may not lead or trail a digit run (§1). A run of
/// several between digits is allowed — the rule is about the boundaries.
fn consume_digit_run(
    cursor: &mut Cursor<u8>,
    radix: u32,
    errors: &mut Vec<(Range<usize>, DiagnosticKind)>,
) -> usize {
    let start = cursor.index();
    let first = cursor.peek(0);
    let mut digits = 0;

    while let Some(byte) = cursor.peek(0) {
        if byte == b'_' {
            cursor.consume();
        } else if is_digit(byte, radix) {
            cursor.consume();
            digits += 1;
        } else {
            break;
        }
    }

    if digits > 0 {
        if first == Some(b'_') {
            errors.push((start..start + 1, DiagnosticKind::LeadingUnderscore));
        }
        if cursor.trail(0) == Some(b'_') {
            let end = cursor.index();
            errors.push((end - 1..end, DiagnosticKind::TrailingUnderscore));
        }
    }

    digits
}

/// An exponent makes a literal a float even with no `.`: `1e10` is a float (§1).
///
/// Three bytes of lookahead, and no backtracking: if what follows `e` isn't an
/// exponent, the `e` is left where it is and becomes the first character of the
/// suffix — so `1e` is an integer with an unknown suffix, not a broken float.
fn consume_exponent(
    cursor: &mut Cursor<u8>,
    is_float: &mut bool,
    errors: &mut Vec<(Range<usize>, DiagnosticKind)>,
) {
    if !matches!(cursor.peek(0), Some(b'e' | b'E')) {
        return;
    }
    let offset = if matches!(cursor.peek(1), Some(b'+' | b'-')) {
        2
    } else {
        1
    };
    if !cursor.peek(offset).is_some_and(|byte| byte.is_ascii_digit()) {
        return;
    }

    *is_float = true;
    for _ in 0..offset {
        cursor.consume();
    }
    consume_digit_run(cursor, 10, errors);
}

fn is_digit(byte: u8, radix: u32) -> bool {
    match radix {
        2 => matches!(byte, b'0' | b'1'),
        8 => (b'0'..=b'7').contains(&byte),
        16 => byte.is_ascii_hexdigit(),
        _ => byte.is_ascii_digit(),
    }
}

fn parse_int(
    digits: &str,
    radix: u32,
    body: &Range<usize>,
    errors: &mut Vec<(Range<usize>, DiagnosticKind)>,
) -> Option<NumberValue> {
    let cleaned = strip_separators(digits);
    if cleaned.is_empty() {
        return None;
    }
    match u128::from_str_radix(&cleaned, radix) {
        Ok(value) => Some(NumberValue::Int(value)),
        // §1 makes an unsuffixed integer an unbounded constant, but only in
        // arithmetic — a *literal* wider than 128 bits has no representation to
        // start from, and pinning it could only ever fail.
        Err(_) => {
            errors.push((body.clone(), DiagnosticKind::IntegerTooLarge));
            None
        }
    }
}

fn parse_float(
    text: &str,
    body: &Range<usize>,
    errors: &mut Vec<(Range<usize>, DiagnosticKind)>,
) -> Option<NumberValue> {
    let cleaned = strip_separators(text);
    match cleaned.parse::<f64>() {
        Ok(value) if value.is_finite() => Some(NumberValue::Float(value)),
        // Rust parses `1e400` as infinity rather than failing. §1 says an
        // inexact float constant rounds rather than erroring, but infinity
        // isn't rounding — it's a value the literal doesn't name.
        Ok(_) => {
            errors.push((body.clone(), DiagnosticKind::FloatOutOfRange));
            None
        }
        Err(_) => None,
    }
}

fn strip_separators(text: &str) -> String {
    text.chars().filter(|&c| c != '_').collect()
}
