//! Glue core IR: concrete syntax tree in, a typed monomorphic program out.
//!
//! This is the representation both back ends consume — the concrete form of
//! goal §both-modes' shared front end, and the thing §comptime names when it
//! says elaboration "lowers to a core IR: typed, monomorphic, and free of
//! comptime, generics, and `Type`". The design and the decisions behind it are
//! in `scratch/core-ir.md`; this crate is that document, executable.
//!
//! ```
//! let parse = parser::parse("let x = 2; x * 21");
//! let lowered = ir::lower(&parse.tree, "let x = 2; x * 21");
//! assert!(lowered.diagnostics.is_empty());
//! println!("{}", ir::dump(&lowered.program));
//! ```
//!
//! # What is here
//!
//! * [`program`] — the instruction set, and the three invariants that shape it.
//! * [`types`] — the type table, nominal for structs and interned otherwise.
//! * [`consts`] — the constant pool, which is where comptime results will land.
//! * [`lower`] — elaboration: name resolution, type checking, A-normal form,
//!   capture analysis, and slot allocation, in one pass over the CST.
//! * [`print`] — the s-expression dump.
//!
//! # What is deliberately absent
//!
//! The IR contains only what lowering can produce today, so there is no node
//! that cannot be exercised. Three things the design anticipates are therefore
//! missing, each additive:
//!
//! * **An instantiation chain on [`program::CstId`]** — §comptime's, once
//!   `comptime` has a token. Provenance is a single CST node until then.
//! * **`CallHost`** — §modules', once a Glue program can declare what it
//!   needs from the host. Until then a program can compute but cannot
//!   observably *do* anything (§statements).
//! * **`Index`, and any collection type** — §types' and §generics'. `Str` is
//!   the only indexable thing the language has, and what indexing it returns
//!   is open.
//!
//! Also unrepresented, and further out: `match` and patterns (§unions), traits
//! and operator overloading (§types, §objects), `Type` and generics
//! (§generics, §comptime).
//!
//! # The one thing to know about the shape
//!
//! Every operand is atomic — a slot or a constant, never a nested computation.
//! §semantics' "operands evaluate left to right" therefore stops being a rule
//! that two back ends must each remember and becomes the order of a statement
//! list. `&&` and `||` do not appear in [`program::BinOp`] for the same
//! reason: they short-circuit, so they lower to [`program::Stmt::If`], and
//! neither back end implements laziness.

pub mod consts;
pub mod cst;
pub mod diagnostic;
pub mod lower;
pub mod print;
pub mod program;
pub mod scan;
pub mod sym;
pub mod types;

#[cfg(test)]
mod tests;

pub use crate::diagnostic::{Diagnostic, DiagnosticKind};
pub use crate::lower::{Lowered, lower};
pub use crate::print::{dump, dump_func};
pub use crate::program::Program;
