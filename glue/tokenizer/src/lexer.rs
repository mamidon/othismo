//! The scanner.
//!
//! A single pass, dispatching on the next byte, taking the longest token that
//! matches. No mode stack, no backtracking, no shared mutable state: the whole
//! of the lexer's state is a cursor and the tokens it has already emitted.
//!
//! Two invariants hold everything together, and both are asserted in tests:
//!
//! * **Tiling.** Each token's span begins where the last one ended, so
//!   concatenating every token's text reproduces the file byte for byte. Only
//!   [`consume_next_token`] builds a span, which is what makes this structural
//!   rather than a thing to remember.
//! * **Progress.** Every scan consumes at least one byte, on every path
//!   including the error paths. Lexing is total — malformed input produces
//!   `Unknown` tokens and diagnostics rather than failing — so a scan that
//!   could consume nothing would be an infinite loop in the language server.

use crate::Tokens;
use crate::cursor::Cursor;
use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::escape::consume_escape;
use crate::number::consume_number;
use crate::span::Span;
use crate::token::{Token, TokenKind};

pub(crate) fn tokenize(source: &str) -> Tokens {
    let mut cursor = Cursor::new(source.as_bytes());
    let mut out = Tokens::default();

    // A leading byte-order mark is skipped, not required (§1). It's emitted as
    // trivia rather than stepped over, so the stream still tiles the file.
    if cursor.eat_seq(&[0xEF, 0xBB, 0xBF]) {
        out.tokens
            .push(Token::new(TokenKind::Whitespace, Span::new(0, 3)));
    }

    while !cursor.at_end() {
        consume_next_token(&mut cursor, source, &mut out);
    }

    out.tokens.push(Token::new(
        TokenKind::Eof,
        Span::empty_at(cursor.index()),
    ));
    out
}

fn consume_next_token(cursor: &mut Cursor<u8>, source: &str, out: &mut Tokens) {
    let start = cursor.index();
    let byte = cursor.peek(0).expect("the caller checked for end of input");

    let kind = match byte {
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
            cursor.consume_while(is_whitespace);
            TokenKind::Whitespace
        }

        b'/' => consume_slash(cursor, &mut out.diagnostics),
        b'"' => consume_string(cursor, &mut out.diagnostics),
        b'\'' => consume_char(cursor, &mut out.diagnostics),
        // `r` is an ordinary identifier unless a quote is glued to it.
        b'r' if cursor.peek(1) == Some(b'"') => consume_raw_string(cursor, &mut out.diagnostics),

        b'0'..=b'9' => consume_number_token(cursor, source, &mut out.diagnostics),
        b'.' => consume_dot(cursor, source, out),

        b'(' => single(cursor, TokenKind::LParen),
        b')' => single(cursor, TokenKind::RParen),
        b'{' => single(cursor, TokenKind::LBrace),
        b'}' => single(cursor, TokenKind::RBrace),
        b'[' => single(cursor, TokenKind::LBracket),
        b']' => single(cursor, TokenKind::RBracket),
        b',' => single(cursor, TokenKind::Comma),
        b';' => single(cursor, TokenKind::Semicolon),
        b'~' => single(cursor, TokenKind::Tilde),

        b':' => {
            cursor.consume();
            // `::` has no construct yet — §13 owns paths — but lexing it means
            // the parser can say so instead of tripping over two colons.
            or_else(cursor, b':', TokenKind::ColonColon, TokenKind::Colon)
        }
        b'+' => {
            cursor.consume();
            or_else(cursor, b'=', TokenKind::PlusEq, TokenKind::Plus)
        }
        b'*' => {
            cursor.consume();
            or_else(cursor, b'=', TokenKind::StarEq, TokenKind::Star)
        }
        b'%' => {
            cursor.consume();
            or_else(cursor, b'=', TokenKind::PercentEq, TokenKind::Percent)
        }
        b'^' => {
            cursor.consume();
            or_else(cursor, b'=', TokenKind::CaretEq, TokenKind::Caret)
        }
        b'!' => {
            cursor.consume();
            or_else(cursor, b'=', TokenKind::BangEq, TokenKind::Bang)
        }
        b'=' => {
            cursor.consume();
            or_else(cursor, b'=', TokenKind::EqEq, TokenKind::Eq)
        }
        b'-' => {
            cursor.consume();
            if cursor.eat(b'=') {
                TokenKind::MinusEq
            } else if cursor.eat(b'>') {
                TokenKind::Arrow
            } else {
                TokenKind::Minus
            }
        }
        b'&' => {
            cursor.consume();
            if cursor.eat(b'&') {
                TokenKind::AmpAmp
            } else if cursor.eat(b'=') {
                TokenKind::AmpEq
            } else {
                TokenKind::Amp
            }
        }
        // `||` with no operand to its left opens a zero-parameter lambda (§5).
        // That's the parser's to sort out; both spellings are one token here.
        b'|' => {
            cursor.consume();
            if cursor.eat(b'|') {
                TokenKind::PipePipe
            } else if cursor.eat(b'=') {
                TokenKind::PipeEq
            } else {
                TokenKind::Pipe
            }
        }
        b'<' => {
            cursor.consume();
            if cursor.eat_seq(b"<=") {
                TokenKind::ShlEq
            } else if cursor.eat(b'<') {
                TokenKind::Shl
            } else if cursor.eat(b'=') {
                TokenKind::Le
            } else {
                TokenKind::Lt
            }
        }
        // No generics yet (§8), so nothing needs `>>` split back apart.
        b'>' => {
            cursor.consume();
            if cursor.eat_seq(b">=") {
                TokenKind::ShrEq
            } else if cursor.eat(b'>') {
                TokenKind::Shr
            } else if cursor.eat(b'=') {
                TokenKind::Ge
            } else {
                TokenKind::Gt
            }
        }

        b if b.is_ascii_alphabetic() || b == b'_' => {
            consume_identifier(cursor, source, &mut out.diagnostics)
        }

        _ => consume_unknown(cursor, source, &mut out.diagnostics),
    };

    debug_assert!(cursor.index() > start, "every scan must consume a byte");
    out.tokens
        .push(Token::new(kind, Span::new(start, cursor.index())));
}

fn single(cursor: &mut Cursor<u8>, kind: TokenKind) -> TokenKind {
    cursor.consume();
    kind
}

fn or_else(cursor: &mut Cursor<u8>, next: u8, longer: TokenKind, shorter: TokenKind) -> TokenKind {
    if cursor.eat(next) { longer } else { shorter }
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

// --- Dots ------------------------------------------------------------------

/// `...`, `..`, a leading-dot float, or field access — in that order.
///
/// Resolving punctuation first is what keeps `0..5` sane: it lexes as
/// `INT DOTDOT INT`, and the float rule never runs. Neither `..` nor `...` has
/// a construct yet (ranges are deferred in §2), but lexing them means a range
/// gets a diagnostic about ranges rather than something incoherent about
/// numbers.
fn consume_dot(cursor: &mut Cursor<u8>, source: &str, out: &mut Tokens) -> TokenKind {
    if cursor.eat_seq(b"...") {
        return TokenKind::DotDotDot;
    }
    if cursor.eat_seq(b"..") {
        return TokenKind::DotDot;
    }

    // §1's one exception to context-free lexing: `.5` is a float unless the
    // preceding significant token could end an expression, in which case the
    // `.` is field access. Decided by the previous *token*, not by adjacency,
    // so `pair. 0` and `pair .0` agree.
    let leading_dot_float = cursor.peek(1).is_some_and(|byte| byte.is_ascii_digit())
        && !previous_can_end_expression(&out.tokens);
    if leading_dot_float {
        return consume_number_token(cursor, source, &mut out.diagnostics);
    }

    cursor.consume();
    TokenKind::Dot
}

fn previous_can_end_expression(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .rev()
        .find(|token| !token.is_trivia())
        .is_some_and(|token| token.kind.can_end_expression())
}

// --- Comments --------------------------------------------------------------

fn consume_slash(cursor: &mut Cursor<u8>, diagnostics: &mut Vec<Diagnostic>) -> TokenKind {
    if cursor.eat_seq(b"///") {
        // Exactly three slashes. A fourth makes it an ordinary comment, so a
        // row of them used as a divider isn't a doc comment attached to
        // nothing.
        let kind = if cursor.peek(0) == Some(b'/') {
            TokenKind::LineComment
        } else {
            TokenKind::DocComment
        };
        consume_to_end_of_line(cursor);
        return kind;
    }
    if cursor.eat_seq(b"//") {
        consume_to_end_of_line(cursor);
        return TokenKind::LineComment;
    }
    if cursor.peek(1) == Some(b'*') {
        return consume_block_comment(cursor, diagnostics);
    }
    cursor.consume();
    or_else(cursor, b'=', TokenKind::SlashEq, TokenKind::Slash)
}

fn consume_to_end_of_line(cursor: &mut Cursor<u8>) {
    cursor.consume_while(|byte| byte != b'\n' && byte != b'\r');
}

/// Block comments nest (§1), so this counts depth rather than looking for the
/// first `*/`.
fn consume_block_comment(cursor: &mut Cursor<u8>, diagnostics: &mut Vec<Diagnostic>) -> TokenKind {
    let start = cursor.index();
    cursor.eat_seq(b"/*");

    let mut depth = 1usize;
    while depth > 0 {
        if cursor.eat_seq(b"/*") {
            depth += 1;
        } else if cursor.eat_seq(b"*/") {
            depth -= 1;
        } else if cursor.consume().is_none() {
            // A block comment legitimately spans lines, so there is nowhere to
            // stop but the end of the file.
            diagnostics.push(Diagnostic::new(
                DiagnosticKind::UnterminatedBlockComment,
                Span::new(start, cursor.index()),
            ));
            break;
        }
    }
    TokenKind::BlockComment
}

// --- Strings and characters ------------------------------------------------

fn consume_string(cursor: &mut Cursor<u8>, diagnostics: &mut Vec<Diagnostic>) -> TokenKind {
    let start = cursor.index();
    if cursor.eat_seq(b"\"\"\"") {
        return consume_multiline_string(cursor, start, diagnostics);
    }

    cursor.consume();
    loop {
        match cursor.peek(0) {
            Some(b'"') => {
                cursor.consume();
                break;
            }
            Some(b'\\') => consume_escape_reporting(cursor, diagnostics),
            // A `"` string does not span lines — that is what `"""` is for — so
            // one missing quote doesn't turn the rest of the file red.
            None | Some(b'\n') | Some(b'\r') => {
                diagnostics.push(Diagnostic::new(
                    DiagnosticKind::UnterminatedString,
                    Span::new(start, cursor.index()),
                ));
                break;
            }
            Some(_) => {
                cursor.consume_utf8();
            }
        }
    }
    TokenKind::Str
}

fn consume_multiline_string(
    cursor: &mut Cursor<u8>,
    start: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> TokenKind {
    loop {
        if cursor.eat_seq(b"\"\"\"") {
            break;
        }
        match cursor.peek(0) {
            None => {
                diagnostics.push(Diagnostic::new(
                    DiagnosticKind::UnterminatedMultilineString,
                    Span::new(start, cursor.index()),
                ));
                break;
            }
            // Escapes are processed inside `"""` (§1) — which also means an
            // escaped quote can't be mistaken for the terminator.
            Some(b'\\') => consume_escape_reporting(cursor, diagnostics),
            Some(_) => {
                cursor.consume_utf8();
            }
        }
    }
    TokenKind::MultilineStr
}

/// `r"…"` — no escapes, so the first quote ends it. There is no `r#"…"#` form,
/// which means a raw string cannot contain a quote at all.
fn consume_raw_string(cursor: &mut Cursor<u8>, diagnostics: &mut Vec<Diagnostic>) -> TokenKind {
    let start = cursor.index();
    cursor.eat_seq(b"r\"");
    loop {
        match cursor.peek(0) {
            Some(b'"') => {
                cursor.consume();
                break;
            }
            None | Some(b'\n') | Some(b'\r') => {
                diagnostics.push(Diagnostic::new(
                    DiagnosticKind::UnterminatedRawString,
                    Span::new(start, cursor.index()),
                ));
                break;
            }
            Some(_) => {
                cursor.consume_utf8();
            }
        }
    }
    TokenKind::RawStr
}

/// `'x'` — exactly one Unicode scalar value (§1). Nothing else in the language
/// uses `'`, so an unterminated one is unambiguously a mistake.
fn consume_char(cursor: &mut Cursor<u8>, diagnostics: &mut Vec<Diagnostic>) -> TokenKind {
    let start = cursor.index();
    cursor.consume();

    let mut characters = 0usize;
    let mut terminated = false;
    loop {
        match cursor.peek(0) {
            Some(b'\'') => {
                cursor.consume();
                terminated = true;
                break;
            }
            None | Some(b'\n') | Some(b'\r') => break,
            Some(b'\\') => {
                consume_escape_reporting(cursor, diagnostics);
                characters += 1;
            }
            Some(_) => {
                cursor.consume_utf8();
                characters += 1;
            }
        }
    }

    let span = Span::new(start, cursor.index());
    let problem = if !terminated {
        Some(DiagnosticKind::UnterminatedChar)
    } else if characters == 0 {
        Some(DiagnosticKind::EmptyChar)
    } else if characters > 1 {
        Some(DiagnosticKind::OverlongChar)
    } else {
        None
    };
    if let Some(kind) = problem {
        diagnostics.push(Diagnostic::new(kind, span));
    }
    TokenKind::Char
}

fn consume_escape_reporting(cursor: &mut Cursor<u8>, diagnostics: &mut Vec<Diagnostic>) {
    let start = cursor.index();
    if let Err(error) = consume_escape(cursor) {
        diagnostics.push(Diagnostic::new(
            error.into(),
            Span::new(start, cursor.index()),
        ));
    }
}

// --- Numbers ---------------------------------------------------------------

fn consume_number_token(
    cursor: &mut Cursor<u8>,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> TokenKind {
    let scan = consume_number(cursor, source);
    for (range, kind) in scan.errors {
        diagnostics.push(Diagnostic::new(kind, Span::new(range.start, range.end)));
    }
    if scan.is_float {
        TokenKind::Float
    } else {
        TokenKind::Int
    }
}

// --- Identifiers and stray text --------------------------------------------

fn consume_identifier(
    cursor: &mut Cursor<u8>,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> TokenKind {
    let start = cursor.index();
    cursor.consume_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');

    // A non-ASCII letter glued to the run means someone wrote a Unicode
    // identifier, which §1 rules out. Take the whole word so the diagnostic
    // covers `café` once, rather than pointing at the `é` and calling the rest
    // a valid identifier.
    if peek_char(cursor, source).is_some_and(is_word_character) {
        consume_word(cursor, source);
        diagnostics.push(Diagnostic::new(
            DiagnosticKind::NonAsciiIdentifier,
            Span::new(start, cursor.index()),
        ));
        return TokenKind::Unknown;
    }

    // Keywords are reserved, not contextual (§1), so this is a lookup and not
    // a question for the parser.
    TokenKind::keyword(&source[start..cursor.index()]).unwrap_or(TokenKind::Ident)
}

fn consume_unknown(
    cursor: &mut Cursor<u8>,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> TokenKind {
    let start = cursor.index();
    let first = peek_char(cursor, source).expect("the caller checked for end of input");

    let kind = if is_word_character(first) {
        consume_word(cursor, source);
        DiagnosticKind::NonAsciiIdentifier
    } else {
        cursor.consume_utf8();
        // Coalesce a run of unrecognized characters, so pasting a paragraph of
        // prose into a buffer produces one squiggle rather than four hundred.
        while let Some(character) = peek_char(cursor, source) {
            let recognized = character.is_whitespace()
                || is_word_character(character)
                || (character.is_ascii() && is_token_start(character as u8));
            if recognized {
                break;
            }
            cursor.consume_utf8();
        }
        DiagnosticKind::UnexpectedCharacter
    };

    diagnostics.push(Diagnostic::new(kind, Span::new(start, cursor.index())));
    TokenKind::Unknown
}

fn peek_char(cursor: &Cursor<u8>, source: &str) -> Option<char> {
    source[cursor.index()..].chars().next()
}

fn consume_word(cursor: &mut Cursor<u8>, source: &str) {
    while peek_char(cursor, source).is_some_and(is_word_character) {
        cursor.consume_utf8();
    }
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Every ASCII byte the dispatcher above has an arm for. Only used to decide
/// where a run of unrecognized text ends.
fn is_token_start(byte: u8) -> bool {
    matches!(byte,
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c
        | b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'0'..=b'9'
        | b'"' | b'\''
        | b'(' | b')' | b'{' | b'}' | b'[' | b']'
        | b',' | b';' | b':' | b'.'
        | b'+' | b'-' | b'*' | b'/' | b'%'
        | b'&' | b'|' | b'^' | b'~' | b'!' | b'=' | b'<' | b'>')
}
