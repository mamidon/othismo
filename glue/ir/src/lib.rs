//! Glue core IR: the typed, monomorphic program both back ends consume.
//!
//! This is the concrete form of goal §both-modes' shared front end, and the
//! thing §comptime names when it says elaboration "lowers to a core IR: typed,
//! monomorphic, and free of comptime, generics, and `Type`". The design and
//! the decisions behind it are in `scratch/core-ir.md`; this crate is that
//! document, executable.
//!
//! # Representation only
//!
//! Nothing here builds a [`Program`] and nothing here runs one. `elab`
//! produces one from a concrete syntax tree; `eval` executes one. That split
//! is what `core-ir.md` decision 2 needs: the interpreter is also the
//! *comptime* engine, so elaboration has to be able to call it, and an
//! evaluator living inside the representation it evaluates would close the
//! loop the wrong way round.
//!
//! ```text
//! tokenizer ← parser ← ir
//!                      ir ← eval
//!               ir, eval ← elab
//!                          elab ← interpreter, lsp
//! ```
//!
//! # What is here
//!
//! * [`program`] — the instruction set, and the three invariants that shape it.
//! * [`types`] — the type table, nominal for structs and interned otherwise.
//! * [`consts`] — the constant pool, which is where comptime results will land.
//! * [`sym`] — interned names, kept for diagnostics and dumps and nothing else.
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
pub mod print;
pub mod program;
pub mod sym;
pub mod types;

pub use crate::print::{dump, dump_func};
pub use crate::program::Program;
