//! Glue parser: tokens in, lossless syntax tree plus diagnostics out.
//!
//! Two properties the tree has to have, whatever else changes:
//!
//! * **Lossless.** Every byte of the source is reachable from the tree,
//!   trivia included, so the same tree can serve formatting later.
//! * **Total.** Parsing never fails. Malformed input produces a tree with
//!   error nodes and a list of diagnostics, because the editor will spend most
//!   of its time looking at half-typed programs.
//!
//! Two representations, one derived from the other in a single pass. The
//! [`syntax`] module holds the concrete tree — a flat vector of events over
//! every token, which is what the language server and the formatter want.
//! Lowering walks it once and produces a typed tree in an arena, where every
//! child slot is already resolved to an index, which is what the type checker
//! and the back ends want. The alternative — typed cursors over the concrete
//! tree — scans on every field access, and the passes downstream of here walk
//! these trees far more often than the editor does.

pub mod builder;
pub mod cursor;
pub mod diagnostic;
pub mod expr;
pub mod stmt;
pub mod syntax;

#[cfg(test)]
mod tests;

pub use crate::builder::{Closed, Mark, TreeBuilder};
pub use crate::cursor::{Cursor, Parse};
pub use crate::diagnostic::{Diagnostic, DiagnosticKind, Severity};
pub use crate::syntax::{Child, Event, NodeId, NodeKind, Tree};

/// Parses `source` as a file.
///
/// A file is a block (§3): statements, optionally ending in an expression with
/// no `;` that is the file's value. So `let x = 2; x * 21` and `42` are both
/// whole programs, which is the entirety of goal §2.1's "a bare expression is
/// a valid program" — the block rule applied to the outermost block, rather
/// than a special case anywhere.
pub fn parse(source: &str) -> Parse {
    let mut cursor = Cursor::new(tokenizer::tokenize(source));
    let file = cursor.open(NodeKind::SourceFile);
    stmt::statements(&mut cursor, tokenizer::TokenKind::Eof);
    cursor.sweep_leftovers();
    cursor.close(file);
    cursor.finish()
}
