//! Reading the concrete syntax tree.
//!
//! `parser` deliberately offers no typed accessors: its tree is a flat event
//! vector, lossless and total, and the shape of a node is documented on
//! [`parser::NodeKind`] rather than encoded in a type. These are the queries
//! elaboration needs, written once here rather than inline at forty call sites.
//!
//! Every one of them tolerates a malformed node. Parsing is total, so a
//! half-typed program reaches here with children missing, and the answer to
//! "what is this `let`'s initializer" has to be `None` rather than a panic.

use parser::{Child, NodeId, NodeKind, Tree};
use tokenizer::{Span, Token, TokenKind};

/// Direct child nodes, in source order. Tokens are skipped.
pub fn nodes(tree: &Tree, node: NodeId) -> Vec<NodeId> {
    tree.children(node)
        .filter_map(|child| match child {
            Child::Node(id) => Some(id),
            Child::Token(_) => None,
        })
        .collect()
}

pub fn has_token(tree: &Tree, node: NodeId, kind: TokenKind) -> bool {
    tree.children(node).any(|child| match child {
        Child::Token(token) => token.kind == kind,
        Child::Node(_) => false,
    })
}

pub fn first_token(tree: &Tree, node: NodeId, kind: TokenKind) -> Option<Token> {
    tree.children(node).find_map(|child| match child {
        Child::Token(token) if token.kind == kind => Some(token),
        _ => None,
    })
}

/// The first identifier directly inside `node` — the name a `fn`, `struct`,
/// `type`, parameter, field, or `NamePat` binds.
pub fn name(tree: &Tree, source: &str, node: NodeId) -> Option<(String, Span)> {
    first_token(tree, node, TokenKind::Ident)
        .map(|token| (token.text(source).to_string(), token.span))
}

/// Every child node that is an expression.
///
/// Used where a node holds a mix of kinds, so that an absent type annotation
/// cannot shift the initializer into its position.
pub fn expr_children(tree: &Tree, node: NodeId) -> Vec<NodeId> {
    nodes(tree, node)
        .into_iter()
        .filter(|id| is_expr(tree.kind(*id)))
        .collect()
}

/// The child node that is a type, if there is one.
pub fn type_child(tree: &Tree, node: NodeId) -> Option<NodeId> {
    nodes(tree, node)
        .into_iter()
        .find(|id| is_type(tree.kind(*id)))
}

pub fn is_expr(kind: NodeKind) -> bool {
    use NodeKind::*;
    matches!(
        kind,
        LiteralExpr
            | NameExpr
            | BlockExpr
            | IfExpr
            | ParenExpr
            | UnitExpr
            | UnaryExpr
            | BinaryExpr
            | CastExpr
            | CallExpr
            | IndexExpr
            | FieldExpr
            | MethodCallExpr
            | StructLitExpr
            | LambdaExpr
    )
}

pub fn is_type(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::NameType | NodeKind::FnType | NodeKind::UnitType
    )
}

/// The forms hoisted to the top of their block, so that mutual recursion works
/// while statements still run in order (§statements, §functions, §scope).
pub fn is_declaration(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::FnDecl | NodeKind::StructDecl | NodeKind::TypeAliasDecl
    )
}
