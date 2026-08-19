//! `glue` — run a file, print what it came to.
//!
//! A file is a block (§3), so its value is its trailing expression, and the
//! whole of goal §2.1's "a bare expression is a valid program" is that a file
//! containing `42` prints `42`. The value is echoed unconditionally, unit
//! included: `()` is a real value (§5), and a program that produced one has not
//! failed.

use std::io::Read;
use std::process::ExitCode;

use interpreter::Error;

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let path = arguments.next();

    if arguments.next().is_some() || matches!(path.as_deref(), Some("-h" | "--help")) {
        eprintln!("usage: glue [FILE]\n\nRuns FILE, or standard input, and prints its value.");
        return ExitCode::FAILURE;
    }

    let (name, source) = match path.as_deref() {
        None | Some("-") => ("<stdin>".to_string(), read_stdin()),
        Some(path) => (path.to_string(), std::fs::read_to_string(path)),
    };
    let source = match source {
        Ok(source) => source,
        Err(problem) => {
            eprintln!("glue: {name}: {problem}");
            return ExitCode::FAILURE;
        }
    };

    match interpreter::run(&source) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(Error::Syntax(errors)) => {
            for error in errors {
                report(&name, &source, error.span.start, error.message);
            }
            ExitCode::FAILURE
        }
        Err(Error::Runtime(error)) => {
            report(&name, &source, error.span.start, &error.to_string());
            ExitCode::FAILURE
        }
    }
}

fn read_stdin() -> std::io::Result<String> {
    let mut source = String::new();
    std::io::stdin().read_to_string(&mut source)?;
    Ok(source)
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
