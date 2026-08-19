//! What stops a program mid-flight.
//!
//! The list is short, and complete, and that is the point of having elaborated
//! first.
//! Everything this crate used to refuse — an unbound name, a condition that
//! isn't `bool`, the wrong number of arguments, `1 + 1.5` — is an
//! [`ir::Diagnostic`] before the program runs. What is left is §2's traps:
//! the operations that are well typed and still have no answer.
//!
//! Every trap is fatal. §9 hasn't decided what a trap is or whether one is
//! recoverable, so the interpreter does the one thing that can't be wrong in
//! advance of that decision: it stops, and says where.
//!
//! # Two types, one shape
//!
//! [`Trap`] carries a [`CstId`] — core IR's provenance, which is what the
//! executor has. [`RuntimeError`] carries a [`Span`], which is what a person
//! reading a terminal needs. The conversion happens in [`crate::run`], the one
//! place that still holds the tree. Everything below the seam talks about IR
//! nodes; everything above it talks about source.

use std::fmt;

use ir::program::CstId;
use tokenizer::Span;

/// A trap, located where the executor can locate it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Trap {
    pub kind: TrapKind,
    pub at: CstId,
}

impl Trap {
    pub fn new(kind: TrapKind, at: CstId) -> Trap {
        Trap { kind, at }
    }
}

/// A trap, located where a person can read it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeError {
    pub kind: TrapKind,
    pub span: Span,
}

impl RuntimeError {
    pub fn new(kind: TrapKind, span: Span) -> RuntimeError {
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
pub enum TrapKind {
    /// §2: overflow is an error, not a wrap. wasm's native behaviour is silent
    /// wrapping, so this is a check the interpreter pays for deliberately —
    /// and §1's widths are what make it reachable at all.
    Overflow {
        operator: &'static str,
        ty: String,
    },
    DividedByZero,
    /// §2: `as` is explicit and trapping. Truncation and rounding are defined
    /// behaviour rather than traps, so this is the conversion with no
    /// representable answer — an integer too wide for its target, or a float
    /// past the edges of one.
    CastOutOfRange {
        value: String,
        ty: String,
    },
    /// §5: there is no tail-call guarantee, so deep recursion exhausts the
    /// stack and traps. This is that trap, raised at a depth the host stack
    /// still has room for rather than by falling off it.
    RecursionLimit,
}

impl fmt::Display for TrapKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use TrapKind::*;
        match self {
            Overflow { operator, ty } => write!(f, "`{operator}` overflowed `{ty}`"),
            DividedByZero => f.write_str("divided by zero"),
            CastOutOfRange { value, ty } => write!(f, "{value} does not fit in `{ty}`"),
            RecursionLimit => f.write_str(
                "recursion went too deep — there are no tail calls, so an unbounded recursion needs a loop or a worklist"
            ),
        }
    }
}
