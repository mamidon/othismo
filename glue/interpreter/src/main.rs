//! `glue` — run a file, or look at what a stage of the pipeline made of it.
//!
//! Two subcommands, because the tool does two things and neither is a mode of
//! the other:
//!
//! ```text
//! glue eval [FILE]          run FILE and print the value it came to
//! glue dump <FILE> <STAGE>  print the s-expression STAGE made of FILE
//! ```
//!
//! `eval` is goal §one-language's "a bare expression is a valid program": a
//! file is a block (§statements), so its value is its trailing expression, and
//! a file containing `42` prints `42`. The value is echoed unconditionally,
//! unit included — `()` is a real value (§functions), and a program that
//! produced one has not failed.
//!
//! `dump` is the debugging view, and it replaces the separate `glue-ir`
//! binary's job. The stages are [`Stage::ALL`]; `glue dump --help` lists them.
//!
//! **The artifact goes to stdout and every diagnostic to stderr**, so a
//! harness can capture them apart, and so `glue dump f.glue ir | less` shows
//! the IR rather than the IR interleaved with complaints about it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use interpreter::{Error, Stage};

#[derive(Parser)]
#[command(
    name = "glue",
    version,
    about = "Run Glue, or look at how it compiles.",
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a file and print the value it came to.
    Eval {
        /// The file to run. Omitted, or `-`, reads standard input.
        file: Option<PathBuf>,
    },
    /// Print what a stage of the pipeline made of a file.
    ///
    /// Every stage renders as an s-expression. A stage prints even when an
    /// earlier one complained — a half-parsed tree or a poisoned IR is
    /// usually the thing worth looking at — except `value`, which will not run
    /// a program that elaboration reported on.
    Dump {
        /// The file to read. `-` reads standard input.
        file: PathBuf,
        /// The stage to stop at.
        stage: Stage,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Eval { file } => {
            let file = file.unwrap_or_else(|| PathBuf::from("-"));
            let Some((name, source)) = read(&file) else {
                return ExitCode::FAILURE;
            };
            eval(&name, &source)
        }
        Command::Dump { file, stage } => {
            let Some((name, source)) = read(&file) else {
                return ExitCode::FAILURE;
            };
            dump(&name, &source, stage)
        }
    }
}

fn eval(name: &str, source: &str) -> ExitCode {
    match interpreter::run(source) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(Error::Syntax(errors)) => {
            for error in errors {
                report(name, source, error.span.start, error.message);
            }
            ExitCode::FAILURE
        }
        // §inference and §scope's, and the reason a program can now fail
        // without having run at all. Every one of them, in source order,
        // because a person fixing a file wants the list rather than the first
        // line of it — and sorted here rather than in `elab`, whose emission
        // order is what the elaboration tests pin. A `fn` body is lowered at
        // the end of its block rather than at its position, so emission order
        // is not source order for a file with functions in it.
        Err(Error::Elaboration(mut diagnostics)) => {
            diagnostics.sort_by_key(|diagnostic| diagnostic.span.start);
            for diagnostic in diagnostics {
                report(name, source, diagnostic.span.start, &diagnostic.message());
            }
            ExitCode::FAILURE
        }
        Err(Error::Runtime(error)) => {
            report(name, source, error.span.start, &error.to_string());
            ExitCode::FAILURE
        }
    }
}

fn dump(name: &str, source: &str, stage: Stage) -> ExitCode {
    let dumped = interpreter::dump(source, stage);

    if let Some(artifact) = dumped.artifact {
        println!("{artifact}");
    }
    for problem in &dumped.problems {
        report(name, source, problem.span.start, &problem.message);
    }

    if dumped.problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The file's name — for diagnostics — and its contents, or `None` once the
/// reason it could not be read has been reported.
fn read(path: &Path) -> Option<(String, String)> {
    if path == Path::new("-") {
        let mut source = String::new();
        return match std::io::stdin().read_to_string(&mut source) {
            Ok(_) => Some(("<stdin>".to_string(), source)),
            Err(problem) => {
                eprintln!("glue: <stdin>: {problem}");
                None
            }
        };
    }

    let name = path.display().to_string();
    match std::fs::read_to_string(path) {
        Ok(source) => Some((name, source)),
        Err(problem) => {
            eprintln!("glue: {name}: {problem}");
            None
        }
    }
}

/// `file:line:column: message`, which is what an editor and a terminal both
/// know how to jump to.
fn report(name: &str, source: &str, offset: u32, message: &str) {
    let (line, column) = position(source, offset as usize);
    eprintln!("{name}:{line}:{column}: {message}");
}

/// A span is a byte offset (§ the tokenizer), and a person reads line and
/// column — 1-based, and counted in characters rather than bytes, because that
/// is what a terminal shows.
fn position(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let mut line = 1;
    let mut line_start = 0;
    for (index, byte) in source.bytes().enumerate().take(offset) {
        if byte == b'\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    (line, source[line_start..offset].chars().count() + 1)
}
