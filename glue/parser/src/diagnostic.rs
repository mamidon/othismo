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
    /// The name a `let`, `fn`, `struct`, `type`, or parameter binds.
    ExpectedName,
    ExpectedSemicolon,
    ExpectedColon,
    /// `let x;` — §3 requires an initializer, so there is no declare-then-assign
    /// and no definite-assignment analysis to specify.
    ExpectedInitializer,
    /// §5 requires parameter types: signatures are annotated, bodies inferred.
    ExpectedParameterType,

    // ---- Unclosed things --------------------------------------------------
    ExpectedOpeningParen,
    ExpectedClosingParen,
    ExpectedClosingBracket,
    ExpectedOpeningBrace,
    ExpectedClosingBrace,
    /// `(x)` with no `->` after it — a lambda whose body never arrived.
    ExpectedLambdaBody,

    // ---- Misuse -----------------------------------------------------------
    /// `a < b < c`. §2 makes comparison non-associative so that the error
    /// names the actual mistake, rather than letting it fail later as a
    /// `bool` compared against a number.
    ChainedComparison,
    /// A `;` where no statement preceded it. §3 has no empty statement.
    StraySemicolon,

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
            ExpectedName => "expected a name",
            ExpectedSemicolon => "expected a `;`",
            ExpectedColon => "expected a `:`",
            ExpectedInitializer => "a binding must have an initializer — write `let x = …`",
            ExpectedParameterType => "expected `:` and a type — every parameter is annotated",
            ExpectedOpeningParen => "expected a `(`",
            ExpectedClosingParen => "expected a closing `)`",
            ExpectedClosingBracket => "expected a closing `]`",
            ExpectedOpeningBrace => "expected a `{`",
            ExpectedClosingBrace => "expected a closing `}`",
            ExpectedLambdaBody => "expected `->` and a body",
            ChainedComparison => {
                "comparison operators cannot be chained — parenthesize, or use `&&`"
            }
            StraySemicolon => "unnecessary `;` — there is no empty statement",
            UnexpectedInput => "unexpected input",
        }
    }

    pub fn severity(&self) -> Severity {
        // Every grammatical problem is an error today. `Severity` stays
        // because the language server wants one uniform path, and because the
        // warnings that lint-shaped checks will want go through it.
        Severity::Error
    }
}
