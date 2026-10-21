//! `glue/syntaxTree` — the parse tree, for a client that wants to draw it.
//!
//! Not part of LSP. The protocol has no request for "what does your tree look
//! like", because no ordinary editor feature needs one; this exists so the
//! extension can render the tree in a panel and tint the source by depth,
//! which is a debugging view rather than a language feature.
//!
//! The whole tree, every time, trivia included. Files are small, and the point
//! of a lossless tree is being able to ask where a comment attached — which
//! only works if the comment is in the answer.

use parser::{Child, NodeId, Tree};
use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::{Range, TextDocumentIdentifier};

use crate::line_index::LineIndex;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyntaxTreeParams {
    pub text_document: TextDocumentIdentifier,
}

/// One node or token, with the tokens under it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyntaxNodeInfo {
    /// The `NodeKind` for a node, the `TokenKind` for a token.
    pub kind: String,
    /// Tokens are leaves and carry `text`; nodes have `children`.
    pub is_token: bool,
    pub range: Range,
    pub text: Option<String>,
    pub children: Vec<SyntaxNodeInfo>,
}

pub fn describe(tree: &Tree, source: &str, index: &LineIndex) -> SyntaxNodeInfo {
    node(tree, tree.root(), source, index)
}

fn node(tree: &Tree, id: NodeId, source: &str, index: &LineIndex) -> SyntaxNodeInfo {
    SyntaxNodeInfo {
        kind: format!("{:?}", tree.kind(id)),
        is_token: false,
        range: range_of(tree.span(id), index),
        text: None,
        children: tree
            .children(id)
            .map(|child| match child {
                Child::Node(child) => node(tree, child, source, index),
                Child::Token(token) => SyntaxNodeInfo {
                    kind: format!("{:?}", token.kind),
                    is_token: true,
                    range: range_of(token.span, index),
                    text: Some(token.text(source).to_string()),
                    children: Vec::new(),
                },
            })
            .collect(),
    }
}

fn range_of(span: tokenizer::Span, index: &LineIndex) -> Range {
    let (start_line, start_column) = index.line_col(span.start as usize);
    let (end_line, end_column) = index.line_col(span.end as usize);
    Range {
        start: tower_lsp::lsp_types::Position::new(start_line, start_column),
        end: tower_lsp::lsp_types::Position::new(end_line, end_column),
    }
}
