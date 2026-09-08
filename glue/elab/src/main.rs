//! `glue-ir` — parse a file and print its core IR.
//!
//! A debugging tool, and for now the only way to look at the IR. It reads a
//! path or standard input, so `glue-ir examples/hello.glue` and
//! `echo 'let x = 2; x * 21' | glue-ir` both work.

use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = std::env::args().nth(1);
    let source = match &path {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("{path}: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            let mut source = String::new();
            if let Err(error) = std::io::stdin().read_to_string(&mut source) {
                eprintln!("stdin: {error}");
                return ExitCode::FAILURE;
            }
            source
        }
    };

    let name = path.as_deref().unwrap_or("<stdin>");
    let mut failed = false;

    // Lexical and grammatical problems first, and in source order. A half
    // parsed tree is what the language server wants and what elaboration does
    // not, so nothing below runs if the parse went wrong.
    let lexed = tokenizer::tokenize(&source);
    let parse = parser::parse(&source);
    let mut syntax: Vec<(u32, String)> = lexed
        .diagnostics
        .iter()
        .map(|d| (d.span.start, d.message().to_string()))
        .chain(
            parse
                .diagnostics
                .iter()
                .map(|d| (d.span.start, d.message().to_string())),
        )
        .collect();
    syntax.sort_by_key(|(start, _)| *start);
    for (start, message) in &syntax {
        let (line, column) = position(&source, *start);
        eprintln!("{name}:{line}:{column}: {message}");
        failed = true;
    }
    if failed {
        return ExitCode::FAILURE;
    }

    let lowered = elab::lower(&parse.tree, &source);
    for diagnostic in &lowered.diagnostics {
        let (line, column) = position(&source, diagnostic.span.start);
        eprintln!("{name}:{line}:{column}: {}", diagnostic.message());
        failed = true;
    }

    println!("{}", ir::dump(&lowered.program));

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn position(source: &str, offset: u32) -> (usize, usize) {
    let offset = offset as usize;
    let before = &source[..offset.min(source.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map(str::len).unwrap_or(0) + 1;
    (line, column)
}
