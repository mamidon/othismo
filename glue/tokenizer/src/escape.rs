//! Escape sequences, understood once.
//!
//! Both the lexer and the literal decoder need to know how far an escape
//! reaches — the lexer to size the token and place a diagnostic, the decoder to
//! produce the character. Two implementations of that would drift, and the
//! drift would show up as the interpreter and the compiler disagreeing about
//! what a string says, which is exactly what design goal §2.2 forbids. So it
//! lives here, and both callers drive the same function.

use crate::cursor::Cursor;
use crate::diagnostic::DiagnosticKind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EscapeError {
    Unknown,
    MissingBrace,
    Unterminated,
    Empty,
    TooLong,
    InvalidDigit,
    InvalidScalar,
}

impl From<EscapeError> for DiagnosticKind {
    fn from(error: EscapeError) -> DiagnosticKind {
        match error {
            EscapeError::Unknown => DiagnosticKind::UnknownEscape,
            EscapeError::MissingBrace => DiagnosticKind::UnicodeEscapeMissingBrace,
            EscapeError::Unterminated => DiagnosticKind::UnicodeEscapeUnterminated,
            EscapeError::Empty => DiagnosticKind::UnicodeEscapeEmpty,
            EscapeError::TooLong => DiagnosticKind::UnicodeEscapeTooLong,
            EscapeError::InvalidDigit => DiagnosticKind::UnicodeEscapeInvalidDigit,
            EscapeError::InvalidScalar => DiagnosticKind::UnicodeEscapeInvalidScalar,
        }
    }
}

/// Consume one escape sequence. The cursor must sit on the backslash.
///
/// Always consumes at least the backslash, so a caller looping over a string
/// body cannot get stuck. The escapes are §1's, and there are no others.
pub(crate) fn consume_escape(cursor: &mut Cursor<u8>) -> Result<char, EscapeError> {
    debug_assert_eq!(cursor.peek(0), Some(b'\\'));
    cursor.consume();

    let Some(byte) = cursor.peek(0) else {
        return Err(EscapeError::Unknown);
    };

    let simple = match byte {
        b'n' => Some('\n'),
        b'r' => Some('\r'),
        b't' => Some('\t'),
        b'0' => Some('\0'),
        b'\\' => Some('\\'),
        b'"' => Some('"'),
        b'\'' => Some('\''),
        _ => None,
    };
    if let Some(character) = simple {
        cursor.consume();
        return Ok(character);
    }

    if byte != b'u' {
        // Take the whole character, so the diagnostic covers `\é` rather than
        // half of it.
        cursor.consume_utf8();
        return Err(EscapeError::Unknown);
    }

    cursor.consume();
    if !cursor.eat(b'{') {
        return Err(EscapeError::MissingBrace);
    }

    let mut digits = 0usize;
    let mut value = 0u32;
    loop {
        match cursor.peek(0) {
            Some(b'}') => {
                cursor.consume();
                break;
            }
            Some(byte) if byte.is_ascii_hexdigit() => {
                cursor.consume();
                // Saturate rather than overflow; `TooLong` reports it below,
                // and a value that wide is out of range regardless.
                value = value
                    .saturating_mul(16)
                    .saturating_add((byte as char).to_digit(16).unwrap());
                digits += 1;
            }
            // Don't run past the end of the line looking for a `}` — a missing
            // brace shouldn't swallow the rest of the file.
            None | Some(b'\n') | Some(b'\r') => return Err(EscapeError::Unterminated),
            Some(_) => return Err(EscapeError::InvalidDigit),
        }
    }

    if digits == 0 {
        return Err(EscapeError::Empty);
    }
    if digits > 6 {
        return Err(EscapeError::TooLong);
    }
    // `char::from_u32` rejects surrogates and anything above 10FFFF, which is
    // exactly §1's rule: a `\u{…}` cannot name a surrogate code point.
    char::from_u32(value).ok_or(EscapeError::InvalidScalar)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escape(text: &str) -> (Result<char, EscapeError>, usize) {
        let mut cursor = Cursor::new(text.as_bytes());
        let result = consume_escape(&mut cursor);
        (result, cursor.index())
    }

    #[test]
    fn simple_escapes() {
        assert_eq!(escape(r"\n"), (Ok('\n'), 2));
        assert_eq!(escape(r"\t"), (Ok('\t'), 2));
        assert_eq!(escape(r"\0"), (Ok('\0'), 2));
        assert_eq!(escape(r"\\"), (Ok('\\'), 2));
        assert_eq!(escape("\\\""), (Ok('"'), 2));
        assert_eq!(escape(r"\'"), (Ok('\''), 2));
    }

    #[test]
    fn unicode_escapes() {
        assert_eq!(escape(r"\u{41}"), (Ok('A'), 6));
        assert_eq!(escape(r"\u{1F600}"), (Ok('\u{1F600}'), 9));
    }

    #[test]
    fn rejects_surrogates_and_out_of_range() {
        assert_eq!(escape(r"\u{D800}").0, Err(EscapeError::InvalidScalar));
        assert_eq!(escape(r"\u{110000}").0, Err(EscapeError::InvalidScalar));
    }

    #[test]
    fn malformed_unicode_escapes() {
        assert_eq!(escape(r"\u41").0, Err(EscapeError::MissingBrace));
        assert_eq!(escape(r"\u{}").0, Err(EscapeError::Empty));
        assert_eq!(escape(r"\u{1234567}").0, Err(EscapeError::TooLong));
        assert_eq!(escape(r"\u{12g}").0, Err(EscapeError::InvalidDigit));
        assert_eq!(escape("\\u{12\nx").0, Err(EscapeError::Unterminated));
    }

    #[test]
    fn unknown_escape_takes_the_whole_character() {
        assert_eq!(escape(r"\q"), (Err(EscapeError::Unknown), 2));
        // `é` is two bytes; the escape covers both.
        assert_eq!(escape("\\é"), (Err(EscapeError::Unknown), 3));
    }

    #[test]
    fn always_advances() {
        assert_eq!(escape("\\").1, 1);
    }
}
