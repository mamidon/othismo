//! The parser's position in the token stream, and its handle on the builder.
//!
//! Two views of the same tokens, which is what keeps the grammar readable and
//! the tree lossless at the same time. [`Cursor::nth`] and everything built on
//! it see only significant tokens, so no rule ever mentions whitespace.
//! [`Cursor::bump`] emits the trivia it passes over on its way, so every byte
//! still lands in the tree.
//!
//! That also settles trivia attachment for free, and settles it the way people
//! expect: trivia is flushed when the *next* significant token is consumed, by
//! which point the node that token belongs to is already open. A comment above
//! a declaration therefore lands inside it. Trailing trivia lands in the
//! enclosing node, which is the conventional answer and the only one available
//! without lookahead.

use tokenizer::{Span, Token, TokenKind, Tokens};

use crate::builder::{Closed, Mark, TreeBuilder};
use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::syntax::{NodeKind, Tree};

/// A parsed file, and everything that went wrong producing it. Both, always —
/// parsing is total, so there is no error case in which the tree is absent.
pub struct Parse {
    pub tree: Tree,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct Cursor {
    tokens: Vec<Token>,
    /// Index into `tokens`, trivia included — the next token not yet emitted.
    pos: usize,
    builder: TreeBuilder,
    diagnostics: Vec<Diagnostic>,
}

impl Cursor {
    pub fn new(lexed: Tokens) -> Cursor {
        debug_assert!(
            lexed
                .tokens
                .last()
                .is_some_and(|last| last.kind == TokenKind::Eof),
            "the token stream must end with Eof"
        );
        Cursor {
            tokens: lexed.tokens,
            pos: 0,
            builder: TreeBuilder::new(),
            diagnostics: Vec::new(),
        }
    }

    // ---- Looking ----------------------------------------------------------

    /// The kind of the `n`th significant token from here. Past the end this is
    /// [`TokenKind::Eof`], so lookahead never needs a bounds check.
    pub fn nth(&self, n: usize) -> TokenKind {
        self.significant().nth(n).map_or(TokenKind::Eof, |t| t.kind)
    }

    pub fn peek(&self) -> TokenKind {
        self.nth(0)
    }

    pub fn at(&self, kind: TokenKind) -> bool {
        self.peek() == kind
    }

    pub fn at_eof(&self) -> bool {
        self.at(TokenKind::Eof)
    }

    /// Where the next significant token starts. What a "expected X here"
    /// diagnostic points at.
    pub fn span(&self) -> Span {
        self.significant()
            .next()
            .map_or_else(|| Span::empty_at(0), |token| token.span)
    }

    fn significant(&self) -> impl Iterator<Item = Token> + '_ {
        self.tokens[self.pos..]
            .iter()
            .copied()
            .filter(|token| !token.is_trivia())
    }

    // ---- Consuming --------------------------------------------------------

    /// Emits the next significant token, and the trivia in front of it.
    ///
    /// Never consumes [`TokenKind::Eof`]: it has an empty span and exists only
    /// so lookahead has no special case, so putting it in the tree would add a
    /// leaf standing for no text.
    pub fn bump(&mut self) {
        self.flush_trivia();
        if self.tokens[self.pos].kind == TokenKind::Eof {
            return;
        }
        self.builder.token(self.tokens[self.pos]);
        self.pos += 1;
    }

    pub fn eat(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            return true;
        }
        false
    }

    /// Consumes `kind`, or reports `on_missing` at the current position and
    /// consumes nothing. The caller carries on either way.
    pub fn expect(&mut self, kind: TokenKind, on_missing: DiagnosticKind) {
        if !self.eat(kind) {
            self.error(on_missing);
        }
    }

    fn flush_trivia(&mut self) {
        while self.tokens[self.pos].is_trivia() {
            self.builder.token(self.tokens[self.pos]);
            self.pos += 1;
        }
    }

    // ---- Building ---------------------------------------------------------

    pub fn open(&mut self, kind: NodeKind) -> Mark {
        self.builder.open(kind)
    }

    pub fn open_before(&mut self, closed: Closed, kind: NodeKind) -> Mark {
        self.builder.open_before(closed, kind)
    }

    pub fn close(&mut self, mark: Mark) -> Closed {
        self.builder.close(mark)
    }

    /// An empty error node at the current position, for something the grammar
    /// required and the source doesn't have. Reports `kind` as well.
    pub fn error_node(&mut self, kind: DiagnosticKind) -> Closed {
        self.error(kind);
        let mark = self.open(NodeKind::Error);
        self.close(mark)
    }

    pub fn error(&mut self, kind: DiagnosticKind) {
        let span = self.span();
        self.diagnostics.push(Diagnostic::new(kind, span));
    }

    // ---- Finishing --------------------------------------------------------

    /// Consumes everything the grammar didn't, into a trailing error node.
    ///
    /// This is what makes losslessness a property of the parser rather than of
    /// the parser being *correct*: however badly recovery goes, no byte is
    /// dropped. Call it with the root still open, since what it emits has to
    /// land inside it.
    pub fn sweep_leftovers(&mut self) {
        self.flush_trivia();
        if !self.at_eof() {
            self.error(DiagnosticKind::UnexpectedInput);
            let leftovers = self.open(NodeKind::Error);
            while !self.at_eof() {
                self.bump();
            }
            self.close(leftovers);
        }
        // Trailing whitespace, which belongs inside the root like everything
        // else.
        self.flush_trivia();
    }

    /// Hands back the parse. Every node must be closed and every token
    /// consumed — see [`Cursor::sweep_leftovers`] for the latter.
    pub fn finish(self) -> Parse {
        debug_assert!(
            self.tokens[self.pos].kind == TokenKind::Eof,
            "finished with tokens left — sweep_leftovers was not called"
        );
        Parse {
            tree: self.builder.finish(),
            diagnostics: self.diagnostics,
        }
    }
}
