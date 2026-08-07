//! Tests for the tree representation and its builder.

use tokenizer::{Span, Token, TokenKind};

use crate::builder::TreeBuilder;
use crate::syntax::{Child, NodeId, NodeKind, Tree};

fn tok(kind: TokenKind, start: u32, end: u32) -> Token {
    Token::new(kind, Span::new(start as usize, end as usize))
}

/// An s-expression rendering of the tree, which is how every assertion below
/// states its expectation — the shape is the thing under test, and a string is
/// the readable way to say what shape.
fn dump(tree: &Tree) -> String {
    fn node(tree: &Tree, id: NodeId, out: &mut String) {
        out.push('(');
        out.push_str(&format!("{:?}", tree.kind(id)));
        for child in tree.children(id) {
            out.push(' ');
            match child {
                Child::Node(child) => node(tree, child, out),
                Child::Token(token) => out.push_str(&format!("{:?}", token.kind)),
            }
        }
        out.push(')');
    }

    let mut out = String::new();
    node(tree, tree.root(), &mut out);
    out
}

/// `1 + 2`, built the straightforward way.
fn one_plus_two() -> Tree {
    let mut builder = TreeBuilder::new();
    let binary = builder.open(NodeKind::BinaryExpr);

    let left = builder.open(NodeKind::LiteralExpr);
    builder.token(tok(TokenKind::Int, 0, 1));
    builder.close(left);

    builder.token(tok(TokenKind::Plus, 2, 3));

    let right = builder.open(NodeKind::LiteralExpr);
    builder.token(tok(TokenKind::Int, 4, 5));
    builder.close(right);

    builder.close(binary);
    builder.finish()
}

#[test]
fn nests() {
    assert_eq!(
        dump(&one_plus_two()),
        "(BinaryExpr (LiteralExpr Int) Plus (LiteralExpr Int))"
    );
}

#[test]
fn children_skip_subtrees() {
    let tree = one_plus_two();
    let children: Vec<_> = tree.children(tree.root()).collect();
    assert_eq!(children.len(), 3, "the nested Int tokens are not children");
    assert!(matches!(children[1], Child::Token(token) if token.kind == TokenKind::Plus));
}

#[test]
fn spans_come_from_tokens() {
    let tree = one_plus_two();
    assert_eq!(tree.span(tree.root()), Span::new(0, 5));

    let Child::Node(left) = tree.children(tree.root()).next().unwrap() else {
        panic!("first child is a node");
    };
    assert_eq!(tree.span(left), Span::new(0, 1));
}

/// `a + b + c` — the case `open_before` exists for. The operands are read one
/// at a time and each `+` has to wrap everything to its left, so the tree must
/// come out left-associative even though nothing knew that when `a` was read.
#[test]
fn open_before_nests_to_the_left() {
    let mut builder = TreeBuilder::new();

    let name = |builder: &mut TreeBuilder, at: u32| {
        let mark = builder.open(NodeKind::NameExpr);
        builder.token(tok(TokenKind::Ident, at, at + 1));
        builder.close(mark)
    };

    let mut lhs = name(&mut builder, 0);
    for (operator, operand) in [(2, 4), (6, 8)] {
        let binary = builder.open_before(lhs, NodeKind::BinaryExpr);
        builder.token(tok(TokenKind::Plus, operator, operator + 1));
        name(&mut builder, operand);
        lhs = builder.close(binary);
    }

    let tree = builder.finish();
    assert_eq!(
        dump(&tree),
        "(BinaryExpr (BinaryExpr (NameExpr Ident) Plus (NameExpr Ident)) Plus (NameExpr Ident))"
    );
    assert_eq!(tree.span(tree.root()), Span::new(0, 9));
}

/// Trivia is a child like any other, which is the whole of losslessness: the
/// tokens of a tree, read in order, are the source text back.
#[test]
fn trivia_is_a_child() {
    let mut builder = TreeBuilder::new();
    let file = builder.open(NodeKind::SourceFile);
    builder.token(tok(TokenKind::Whitespace, 0, 2));
    let name = builder.open(NodeKind::NameExpr);
    builder.token(tok(TokenKind::Ident, 2, 3));
    builder.close(name);
    builder.close(file);

    let tree = builder.finish();
    assert_eq!(dump(&tree), "(SourceFile Whitespace (NameExpr Ident))");
    assert_eq!(tree.span(tree.root()), Span::new(0, 3));
}

#[test]
fn an_empty_node_is_still_a_node() {
    let mut builder = TreeBuilder::new();
    let file = builder.open(NodeKind::SourceFile);
    let error = builder.open(NodeKind::Error);
    builder.close(error);
    builder.close(file);

    let tree = builder.finish();
    assert_eq!(dump(&tree), "(SourceFile (Error))");
    assert_eq!(tree.children(tree.root()).count(), 1);
}
