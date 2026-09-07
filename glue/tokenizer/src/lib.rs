//! Glue tokenizer: source text in, tokens plus diagnostics out.
//!
//! The language it lexes is a core cut from §lexical of the construct
//! checklist, plus the operators §expressions and §statements spell out.
//! Nothing is lexed that the grammar has no use for: a token that exists only
//! to give the parser something to complain about is a token to add back when
//! the construct arrives.
//!
//! Three properties shape the API:
//!
//! * **Total.** [`tokenize`] never fails. Malformed input produces
//!   [`TokenKind::Unknown`] tokens and diagnostics, because the editor spends
//!   most of its time looking at half-typed programs.
//! * **Lossless.** Whitespace and comments are tokens, and every token's span
//!   begins where the previous one ended — so the token stream reproduces the
//!   file byte for byte. §lexical says comments produce no tokens; the parser
//!   needs trivia to build a lossless tree. Both hold if comments are tokens
//!   the grammar never sees, which is what [`Tokens::significant`] is for.
//!
//! The token set is deliberately smaller than §lexical describes: no doc
//! comments, no raw or multiline strings, no bitwise or shift operators, no
//! compound assignment, no `::`, `..`, or `...`. Those are features to add
//! back when there is a reason to, not features that were tried and rejected.
//! * **Context-free, with one exception.** No lexical decision depends on
//!   parse state. Exactly one depends on the previous token — §lexical's `.5`
//!   rule — and it is written down in [`TokenKind::can_end_expression`] rather
//!   than scattered through the scanner.
//!
//! A token carries no payload. Literal values are decoded on demand by
//! [`literal_value`], so lexing allocates nothing per token and a span is
//! always an index into the original text.
//!
//! ```
//! use tokenizer::{TokenKind, tokenize};
//!
//! let lexed = tokenize("let x = 42;");
//! let kinds: Vec<_> = lexed.significant().map(|token| token.kind).collect();
//! assert_eq!(kinds, [
//!     TokenKind::Let,
//!     TokenKind::Ident,
//!     TokenKind::Equals,
//!     TokenKind::Int,
//!     TokenKind::Semicolon,
//!     TokenKind::Eof,
//! ]);
//! assert!(lexed.diagnostics.is_empty());
//! ```

mod cursor;
mod diagnostic;
mod escape;
mod lexer;
mod literal;
mod number;
mod span;
mod token;

#[cfg(test)]
mod tests;

pub use crate::cursor::Cursor;
pub use crate::diagnostic::{Diagnostic, DiagnosticKind, Severity};
pub use crate::literal::{Literal, NumericType, literal_value};
pub use crate::span::Span;
pub use crate::token::{Token, TokenKind};

/// Everything lexing a file produced.
#[derive(Clone, Default, Debug)]
pub struct Tokens {
    /// Every token, trivia included, in source order and ending with
    /// [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Tokens {
    /// The tokens the grammar sees: everything but whitespace and comments.
    pub fn significant(&self) -> impl Iterator<Item = Token> + '_ {
        self.tokens
            .iter()
            .copied()
            .filter(|token| !token.is_trivia())
    }

    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

/// Lex `source`. Never fails; see the module documentation.
pub fn tokenize(source: &str) -> Tokens {
    lexer::tokenize(source)
}
