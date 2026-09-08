//! The evaluator: core IR in, a value out.
//!
//! The seam goal §both-modes asks for. [`eval`] takes an [`ir::Program`] and
//! nothing else — no tree, no names, no source — so what it computes is what a
//! wasm back end reading the same IR has to compute. [`exec`] holds the walk
//! and the three properties of the IR that shape it.
//!
//! # This is also the comptime engine
//!
//! `core-ir.md` decision 2: rather than a second evaluator over the typed
//! tree, elaboration lowers a body to core IR and runs *this* on it. One
//! evaluator makes a comptime/runtime divergence unrepresentable, which is
//! goal §both-modes' thesis applied one level up.
//!
//! What that decision asks for and this crate does not have yet, because
//! `comptime` has no token: a second **configuration** — a fuel budget, a
//! recursion cap tighter than the stack, host imports denied, `Type` values
//! permitted, and a trap reported as a diagnostic rather than raised. Two
//! configurations, not two programs.
//!
//! # Why this is a crate and not a module of `interpreter`
//!
//! `elab` has to be able to call it (above), and `interpreter` depends on
//! `elab`. Execution below the seam therefore lives here, and the driver that
//! parses, elaborates, runs, and prints stays in `interpreter`.
//!
//! That is also where the two halves of a failure part company. A [`Trap`]
//! carries a `CstId` — core IR's provenance, which is what the executor has.
//! `interpreter::RuntimeError` carries a `Span`, which is what a person
//! reading a terminal needs. Nothing on this side of the seam knows what a
//! source file is.

mod exec;
mod ops;
mod trap;
mod value;

pub use crate::trap::{Trap, TrapKind};
pub use crate::value::{Closure, IntTy, StructShape, StructVal, Value};

use ir::Program;

/// Runs an already-elaborated program.
///
/// Refuses nothing: elaboration is total and reports every problem it finds,
/// so a caller that hands over a program with a diagnostic against it is
/// asking for whatever `TypeDef::Error` happens to run as. `interpreter::run`
/// is the one that checks first.
pub fn eval(program: &Program) -> Result<Value, Trap> {
    exec::Machine::new(program).run()
}
