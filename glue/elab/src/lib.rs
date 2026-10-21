//! Elaboration: concrete syntax tree in, core IR out.
//!
//! Name resolution, type checking, A-normal form, capture analysis, and slot
//! allocation, in one pass over the CST. §comptime says they must be one pass —
//! an annotation can need a call evaluated before its type exists, so they are
//! mutually recursive and admit no ordering. [`lower`] is where that happens
//! and where the argument for it is written down.
//!
//! ```
//! let parse = parser::parse("let x = 2; x * 21");
//! let lowered = elab::lower(&parse.tree, "let x = 2; x * 21");
//! assert!(lowered.diagnostics.is_empty());
//! println!("{}", ir::dump(&lowered.program));
//! ```
//!
//! # Why this is not part of `ir`
//!
//! `core-ir.md` decision 2 makes the interpreter the comptime engine: rather
//! than a second evaluator over the typed tree, elaboration lowers a body to
//! core IR and runs the existing one on it, so comptime and runtime semantics
//! cannot diverge. That makes elaboration a *consumer* of `eval` as well as a
//! producer of `ir`, which is a dependency `ir` cannot hold without a cycle.
//!
//! ```text
//! ir ← eval
//! ir, eval ← elab
//! ```
//!
//! The `eval` dependency is declared and not yet used. It is the reason this
//! crate exists apart from `ir`, and the seam §comptime lands on: a comptime
//! call reaches a callee, the callee is elaborated at the point its comptime
//! parameters are bound, and the result is cached on `(declaration, comptime
//! arguments)`. None of that is built. `comptime` lexes and parses as of
//! 2026-09-07 and elaborates to "comptime is not supported yet" in both of its
//! positions; `Type`, the instantiation cache, and the call into `eval` are
//! what remain.
//!
//! # What is here
//!
//! * [`lower`] — the pass itself.
//! * [`cst`] — the queries elaboration makes of `parser`'s untyped tree.
//! * [`scan`] — the two syntactic pre-passes: is a binding assigned, is it
//!   captured.
//! * [`diagnostic`] — what elaboration complains about.
//!
//! # Elaboration is total
//!
//! A program with an unbound name still produces IR, with `TypeDef::Error`
//! where the answer would have been. That is what the language server wants —
//! `parser` is lossless and total for the same reason — and what an
//! interpreter does not: running a program elaboration reported on would mean
//! guessing what it meant.

pub mod cst;
pub mod diagnostic;
pub mod lower;
pub mod scan;

#[cfg(test)]
mod tests;

pub use crate::diagnostic::{Diagnostic, DiagnosticKind};
pub use crate::lower::{Lowered, lower};
