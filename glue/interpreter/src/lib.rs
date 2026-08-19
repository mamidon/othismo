//! Glue interpreter: source in, a value out.
//!
//! A tree-walking interpreter over the concrete syntax tree — the one
//! [`parser::parse`] produces, with no lowering in between. That is a stopgap
//! and is meant to be: the IL is being designed separately, and when it lands
//! this crate's job becomes walking that instead. Until then the cheapest thing
//! that runs a program is worth more than a second tree to keep in step with
//! the first, and everything decided here — §2's arithmetic, §3's bindings,
//! §4's loop — survives the change of input.
//!
//! # What runs today
//!
//! Expressions, `let`, assignment, blocks, `if`, and `while`: everything that
//! needs neither functions nor a type checker. A file is a block (§3), so its
//! value is its trailing expression, and `let x = 2; x * 21` and `42` are both
//! whole programs.
//!
//! ```
//! use interpreter::{Value, run};
//!
//! assert_eq!(run("let x = 2; x * 21").unwrap(), Value::Int(42));
//! ```
//!
//! Everything else parses and then says it isn't implemented yet, which is a
//! different thing to hear than a syntax error: `fn`, `struct`, `type`,
//! `return`, calls, lambdas, `as`, indexing, and field access.
//!
//! # What is knowingly missing
//!
//! **There are no types.** Every integer is an `i64` and every float an `f64`,
//! so §1's numeric tower is absent, a literal's suffix is read only to tell an
//! integer from a float, and a `let`'s annotation is ignored. §2's rules that
//! don't depend on width — trapping overflow, truncating division, no implicit
//! conversion, no truthiness, no cross-type comparison — all hold. See
//! [`Value`] for the details of what that costs.

mod env;
mod error;
mod eval;
mod ops;
mod value;

#[cfg(test)]
mod tests;

pub use crate::error::{RuntimeError, RuntimeErrorKind};
pub use crate::value::Value;

use std::fmt;

use parser::Tree;
use tokenizer::{Severity, Span};

/// Everything that can stop a program producing a value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    /// The program didn't parse. Every problem found, in source order, because
    /// a caller running a file wants the whole list and not the first one.
    Syntax(Vec<SyntaxError>),
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
            Error::Runtime(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

/// Parses `source` and evaluates it.
///
/// Refuses to run a program with any syntax error in it. A half-parsed tree is
/// exactly what the language server wants and exactly what an interpreter does
/// not: evaluating around an error node means guessing what the missing text
/// said.
pub fn run(source: &str) -> Result<Value, Error> {
    let errors = syntax_errors(source);
    if !errors.is_empty() {
        return Err(Error::Syntax(errors));
    }
    eval(&parser::parse(source).tree, source).map_err(Error::Runtime)
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

/// Evaluates an already-parsed tree.
///
/// The seam the IL will move: this is the only entry point that takes a tree,
/// so swapping the concrete syntax tree for a lowered one is a change to this
/// signature and to the walk behind it, and nothing else.
pub fn eval(tree: &Tree, source: &str) -> Result<Value, RuntimeError> {
    eval::Interpreter::new(tree, source).run()
}
