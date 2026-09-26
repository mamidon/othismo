//! A trap, located where a person can read it.
//!
//! The other half of `eval::Trap`. Below the seam a failure carries a
//! `CstId`, because core IR's provenance is all the executor has; above it a
//! failure carries a [`Span`], because that is what a person reading a
//! terminal needs. [`crate::run`] does the conversion, since it is the one
//! place that still holds the tree.

use std::fmt;

use eval::TrapKind;
use tokenizer::Span;

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
