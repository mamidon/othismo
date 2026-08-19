//! What the interpreter refuses to do, and why.
//!
//! Unlike [`parser::Diagnostic`], these kinds carry data. The parser's are
//! data-free so a message is a `&'static str` on a path the language server
//! runs per keystroke; a runtime error happens once, at the end of an
//! evaluation that is already over, so there is nothing to save by leaving the
//! offending name out of the message.
//!
//! Every error is fatal. §9 hasn't decided what a trap is or whether one is
//! recoverable, so the interpreter does the one thing that can't be wrong in
//! advance of that decision: it stops, and says where.

use std::fmt;

use tokenizer::Span;

/// Something that went wrong while evaluating, and the source it went wrong at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeError {
    pub kind: RuntimeErrorKind,
    pub span: Span,
}

impl RuntimeError {
    pub fn new(kind: RuntimeErrorKind, span: Span) -> RuntimeError {
        RuntimeError { kind, span }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RuntimeErrorKind {
    // ---- Names ------------------------------------------------------------
    UnknownName(String),
    /// §3: `mut` gates mutation. Rebinding with a second `let` is always
    /// allowed, which is what the message points at.
    ImmutableBinding(String),
    /// §3's left-hand side is a *place*. A name is the only one that exists at
    /// this stage — fields and indexes need §6's types.
    NotAPlace,

    // ---- Types ------------------------------------------------------------
    /// §2: there is no truthiness. A condition is `bool` or it is nothing.
    ConditionNotBool(&'static str),
    BinaryTypeMismatch {
        operator: &'static str,
        left: &'static str,
        right: &'static str,
    },
    UnaryTypeMismatch {
        operator: &'static str,
        operand: &'static str,
    },

    // ---- Traps ------------------------------------------------------------
    // §2: overflow is an error, not a wrap. wasm's native behaviour is silent
    // wrapping, so this is a check the interpreter pays for deliberately.
    Overflow(&'static str),
    DividedByZero,
    /// An integer literal too large for the `i64` every integer is today. Not a
    /// language rule — see the note on [`crate::Value`].
    IntegerTooLarge,

    // ---- Not yet ----------------------------------------------------------
    /// A construct that parses and that this stage of the interpreter doesn't
    /// evaluate. Named rather than lumped together, because "functions are not
    /// implemented yet" is a different thing to hear than "syntax error".
    Unsupported(&'static str),
    /// `break` or `continue` with no loop to apply to (§4).
    BreakOutsideLoop,
    ContinueOutsideLoop,
    /// A literal the tokenizer already complained about, or an error node.
    /// Reachable only by evaluating a tree that didn't parse cleanly.
    MalformedProgram,
}

impl fmt::Display for RuntimeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use RuntimeErrorKind::*;
        match self {
            UnknownName(name) => write!(f, "`{name}` is not bound to anything"),
            ImmutableBinding(name) => write!(
                f,
                "`{name}` is not mutable — declare it `let mut {name}`, or rebind it with a new `let`"
            ),
            NotAPlace => f.write_str("this cannot be assigned to — the left of a `=` is a name"),
            ConditionNotBool(found) => write!(
                f,
                "a condition must be a boolean, and this is {found} — there is no truthiness"
            ),
            BinaryTypeMismatch {
                operator,
                left,
                right,
            } => write!(f, "`{operator}` does not apply to {left} and {right}"),
            UnaryTypeMismatch { operator, operand } => {
                write!(f, "`{operator}` does not apply to {operand}")
            }
            Overflow(operator) => write!(f, "`{operator}` overflowed"),
            DividedByZero => f.write_str("divided by zero"),
            IntegerTooLarge => f.write_str("this integer does not fit in 64 bits"),
            Unsupported(what) => write!(f, "{what} is not implemented yet"),
            BreakOutsideLoop => f.write_str("`break` outside a loop"),
            ContinueOutsideLoop => f.write_str("`continue` outside a loop"),
            MalformedProgram => f.write_str("this program did not parse"),
        }
    }
}
