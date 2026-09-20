//! Stopping the pipeline early and printing what the stage made.
//!
//! [`crate::run`] is the whole pipeline with only the value at the end of it
//! visible. This is the other view: stop anywhere, print the artifact, and say
//! what anyone complained about on the way.
//!
//! Three properties, each of which a harness comparing dumps against expected
//! files depends on:
//!
//! * **[`Stage::ALL`] is the pipeline order, written once.** The help text and
//!   anything that enumerates stages read it, so they cannot disagree about
//!   what exists or what runs before what.
//! * **A stage emits even when an earlier stage complained.** A poisoned tree
//!   or a `TypeDef::Error` in the IR is usually the thing you wanted to look
//!   at, and both are produced — parsing and elaboration are total.
//! * **[`Stage::Value`] is the exception**, for the reason [`crate::run`]
//!   refuses the same program: running a file elaboration reported on means
//!   guessing what it meant.
//!
//! Every rendering here is a debugging aid with no normative status — the
//! phrase is `core-ir.md`'s, and it is as true of the token stream and the
//! tree as it is of the IR.

use clap::ValueEnum;
use tokenizer::{Severity, Span};

use crate::RuntimeError;

/// Where a dump stops, in pipeline order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Stage {
    /// The token stream, trivia included.
    Tokens,
    /// The concrete syntax tree, lossless and indented.
    Cst,
    /// Core IR, after elaboration.
    Ir,
    /// The value the program came to.
    Value,
}

impl Stage {
    /// Pipeline order, and the one place it is written down.
    pub const ALL: [Stage; 4] = [Stage::Tokens, Stage::Cst, Stage::Ir, Stage::Value];

    /// The name the command line spells it with.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Tokens => "tokens",
            Stage::Cst => "cst",
            Stage::Ir => "ir",
            Stage::Value => "value",
        }
    }
}

/// Something with a source location that a person should be told about.
///
/// A dump flattens the three stages' diagnostic types into one list because
/// the only thing it does with them is print them in source order, and a
/// reader fixing a file does not care which crate objected.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Problem {
    pub span: Span,
    pub message: String,
}

/// What a dump produced.
pub struct Dump {
    /// The stage's rendering, absent only when the stage declined to run —
    /// which today is [`Stage::Value`] on a program with a diagnostic against
    /// it.
    pub artifact: Option<String>,
    /// Everything found on the way to it, in source order.
    pub problems: Vec<Problem>,
}

/// Runs `source` as far as `stage` and renders what that stage made of it.
///
/// ```
/// use interpreter::{Stage, dump};
///
/// let dumped = dump("let x = 2; x * 21", Stage::Value);
/// assert_eq!(dumped.artifact.unwrap(), "(value u64 42)");
/// assert!(dumped.problems.is_empty());
/// ```
pub fn dump(source: &str, stage: Stage) -> Dump {
    // Lexing happens here rather than through `crate::syntax_errors` because
    // the tokens themselves are an artifact now, and a stage that stopped at
    // the tokens has no business reporting what the grammar thought.
    let lexed = tokenizer::tokenize(source);
    let mut problems: Vec<Problem> = lexed
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity() == Severity::Error)
        .map(|diagnostic| Problem {
            span: diagnostic.span,
            message: diagnostic.message().to_string(),
        })
        .collect();

    if stage == Stage::Tokens {
        return finish(lexed.render(source), problems);
    }

    let parse = parser::parse(source);
    problems.extend(parse.diagnostics.iter().map(|diagnostic| Problem {
        span: diagnostic.span,
        message: diagnostic.message().to_string(),
    }));

    if stage == Stage::Cst {
        return finish(parse.tree.render(source), problems);
    }

    let lowered = elab::lower(&parse.tree, source);
    problems.extend(lowered.diagnostics.iter().map(|diagnostic| Problem {
        span: diagnostic.span,
        message: diagnostic.message().to_string(),
    }));

    if stage == Stage::Ir {
        return finish(ir::dump(&lowered.program), problems);
    }

    if !problems.is_empty() {
        problems.sort_by_key(|problem| problem.span.start);
        return Dump {
            artifact: None,
            problems,
        };
    }

    match eval::eval(&lowered.program) {
        Ok(value) => finish(format!("(value {} {value})", value.type_name()), problems),
        // The trap knows which IR node it came from; only here is the tree
        // still around to say where that node is in the file.
        Err(trap) => {
            let error = RuntimeError::new(trap.kind, crate::span(&parse.tree, trap.at));
            problems.push(Problem {
                span: error.span,
                message: error.to_string(),
            });
            Dump {
                artifact: None,
                problems,
            }
        }
    }
}

/// Source order, and stable — so two problems at the same offset stay in the
/// order the stage that found them emitted them.
fn finish(artifact: String, mut problems: Vec<Problem>) -> Dump {
    problems.sort_by_key(|problem| problem.span.start);
    Dump {
        artifact: Some(artifact),
        problems,
    }
}
