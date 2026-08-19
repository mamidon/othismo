//! Glue interpreter: source in, a value out.
//!
//! Source is parsed, elaborated to core IR by `ir`, and executed. The
//! interpreter used to walk the concrete syntax tree instead; that was a
//! bring-up artifact, and it sat at the wrong end of the pipeline — name
//! resolution, coercion, and evaluation order accumulated in it and lived
//! nowhere else, which is exactly the divergence goal §2.2 names. They live in
//! elaboration now, where the wasm back end will read them too.
//!
//! ```
//! use interpreter::{Value, run};
//!
//! assert_eq!(run("let x = 2; x * 21").unwrap(), Value::u64(42));
//! ```
//!
//! # What runs today
//!
//! The scalar core: expressions, `let`, assignment, blocks, `if`, `while`, and
//! §5's `fn` declarations, calls, and `return`. A file is a block (§3), so its
//! value is its trailing expression, and `let x = 2; x * 21` and `42` are both
//! whole programs.
//!
//! Closures, cells, structs, field access, and `as` elaborate but do not
//! execute yet: each is an IR node the executor reports as
//! [`TrapKind::Unsupported`] rather than running. They are the next two steps
//! rather than open questions.
//!
//! # What changed when the IR arrived
//!
//! **There are types.** §1's numeric tower is real: a literal is pinned to a
//! width, `255u8 + 1` traps where `255u16 + 1` does not, and an annotation is
//! checked rather than read past. See [`Value`].
//!
//! **Most of what used to be a runtime error is a compile error.** An unbound
//! name, a condition that isn't `bool`, the wrong number of arguments, `1 +
//! 1.5` — all of them are [`ir::Diagnostic`]s now, reported before anything
//! runs. §2's "constant expressions are checked at compile time rather than
//! trapping" moves further still: `1 / 0` is a diagnostic, and the trap needs a
//! value that isn't constant.
//!
//! What is left at run time is §2's traps: overflow, division by zero, and the
//! recursion limit.

mod error;
mod exec;
mod ops;
mod value;

#[cfg(test)]
mod tests;

pub use crate::error::{RuntimeError, Trap, TrapKind};
pub use crate::value::{IntTy, Value};

use std::fmt;

use ir::Program;
use parser::Tree;
use tokenizer::{Severity, Span};

/// Everything that can stop a program producing a value.
///
/// Three stages, three shapes. A syntax error is a list because a caller
/// running a file wants every problem in it; so is an elaboration error, for
/// the same reason. A trap is one, because it happened.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    Syntax(Vec<SyntaxError>),
    /// Name resolution and type checking (§10, §12), in source order.
    Elaboration(Vec<ir::Diagnostic>),
    Runtime(RuntimeError),
}

/// A lexical or grammatical problem, flattened.
///
/// [`tokenizer::Diagnostic`] and [`parser::Diagnostic`] are separate types on
/// purpose — a lexical problem and a grammatical one have nothing to say to
/// each other — but a caller that just wants to print them wants one list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SyntaxError {
    pub message: &'static str,
    pub span: Span,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax(errors) => {
                for (index, error) in errors.iter().enumerate() {
                    if index > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}", error.message)?;
                }
                Ok(())
            }
            Error::Elaboration(diagnostics) => {
                for (index, diagnostic) in diagnostics.iter().enumerate() {
                    if index > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}", diagnostic.message())?;
                }
                Ok(())
            }
            Error::Runtime(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

/// Parses `source`, elaborates it, and runs it.
///
/// Refuses to run a program with a diagnostic against it, at either stage. A
/// half-parsed tree is exactly what the language server wants and exactly what
/// an interpreter does not, and the same is true one stage later: elaboration
/// is total, so a program with an unbound name still produces IR, with
/// `TypeDef::Error` where the answer would have been. Running that would mean
/// guessing what the program meant.
pub fn run(source: &str) -> Result<Value, Error> {
    let errors = syntax_errors(source);
    if !errors.is_empty() {
        return Err(Error::Syntax(errors));
    }

    let parse = parser::parse(source);
    let lowered = ir::lower(&parse.tree, source);
    if !lowered.diagnostics.is_empty() {
        return Err(Error::Elaboration(lowered.diagnostics));
    }

    // The trap knows which IR node it came from; only here is the tree still
    // around to say where that node is in the file.
    eval(&lowered.program)
        .map_err(|trap| Error::Runtime(RuntimeError::new(trap.kind, span(&parse.tree, trap.at))))
}

/// Every syntax error in `source`, lexical and grammatical, in source order.
///
/// Lexing happens twice — once here and once inside [`parser::parse`] — because
/// `parse` keeps a `Tokens`' tokens and drops its diagnostics. Lexing a file is
/// cheap and silently running a program with a broken escape in it is not.
pub fn syntax_errors(source: &str) -> Vec<SyntaxError> {
    let lexical = tokenizer::tokenize(source)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity() == Severity::Error)
        .map(|diagnostic| SyntaxError {
            message: diagnostic.message(),
            span: diagnostic.span,
        });
    let grammatical = parser::parse(source)
        .diagnostics
        .into_iter()
        .map(|diagnostic| SyntaxError {
            message: diagnostic.message(),
            span: diagnostic.span,
        });

    let mut errors: Vec<_> = lexical.chain(grammatical).collect();
    errors.sort_by_key(|error| error.span.start);
    errors
}

/// Runs an already-elaborated program.
///
/// The seam goal §2.2 asks for: this takes core IR and nothing else, so what it
/// computes is what a wasm back end reading the same IR has to compute. A trap
/// carries the IR node it happened at rather than a span, because the IR is all
/// this side of the seam can see.
pub fn eval(program: &Program) -> Result<Value, Trap> {
    exec::Machine::new(program).run()
}

/// Where a piece of IR came from, in the file.
///
/// The significant extent rather than the whole node: provenance points at a
/// CST node, the tree is lossless, and a node begins at the trivia attached to
/// its first token — so the plain extent of a statement can start a blank line
/// above the statement.
fn span(tree: &Tree, at: ir::program::CstId) -> Span {
    tree.significant_span(at)
}
