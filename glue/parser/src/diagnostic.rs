//! What the parser complains about.
//!
//! Shaped like [`tokenizer::Diagnostic`] on purpose — same `kind` plus `span`,
//! same data-free kinds so a message is a `&'static str` and nothing formats on
//! the path the language server runs per keystroke. The two lists stay separate
//! because a lexical problem and a grammatical one have nothing to say to each
//! other, and a caller that wants both can chain them.

pub use tokenizer::Severity;
use tokenizer::Span;

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

/// Data-free, which is why "expected a closing delimiter" is three variants
/// rather than one carrying a [`tokenizer::TokenKind`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiagnosticKind {
    // ---- Missing things ---------------------------------------------------
    ExpectedExpression,
    ExpectedType,
    /// `.` with nothing usable after it.
    ExpectedFieldName,

    // ---- Unclosed things --------------------------------------------------
    ExpectedClosingParen,
    ExpectedClosingBracket,

    // ---- Misuse -----------------------------------------------------------
    /// `a < b < c`. §2 makes comparison non-associative so that the error
    /// names the actual mistake, rather than letting it fail later as a
    /// `bool` compared against a number.
    ChainedComparison,

    // ---- Leftovers --------------------------------------------------------
    /// Input the parser could not attach to anything. The tokens are kept in
    /// an error node regardless, because the tree stays lossless even when the
    /// parse doesn't.
    UnexpectedInput,
}

impl DiagnosticKind {
    pub fn message(&self) -> &'static str {
        use DiagnosticKind::*;
        match self {
            ExpectedExpression => "expected an expression",
            ExpectedType => "expected a type",
            ExpectedFieldName => "expected a field or method name after `.`",
            ExpectedClosingParen => "expected a closing `)`",
            ExpectedClosingBracket => "expected a closing `]`",
            ChainedComparison => {
                "comparison operators cannot be chained — parenthesize, or use `&&`"
            }
            UnexpectedInput => "unexpected input",
        }
    }

    pub fn severity(&self) -> Severity {
        Severity::Error
    }
}
