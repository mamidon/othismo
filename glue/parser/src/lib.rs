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
pub mod syntax;

#[cfg(test)]
mod tests;

pub use crate::builder::{Closed, Mark, TreeBuilder};
pub use crate::cursor::{Cursor, Parse};
pub use crate::diagnostic::{Diagnostic, DiagnosticKind, Severity};
pub use crate::syntax::{Child, Event, NodeId, NodeKind, Tree};

/// Parses `source` as a single expression.
///
/// The entry point the grammar can currently justify: §3's top level is a
/// block of statements, and statements aren't parsed yet. This becomes
/// `parse`, over a [`NodeKind::SourceFile`] of statements, when they are.
pub fn parse_expression(source: &str) -> Parse {
    let mut cursor = Cursor::new(tokenizer::tokenize(source));
    let file = cursor.open(NodeKind::SourceFile);
    expr::expr(&mut cursor);
    cursor.sweep_leftovers();
    cursor.close(file);
    cursor.finish()
}
