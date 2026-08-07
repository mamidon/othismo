//! Tests for the tree representation, its builder, and expression parsing.

use tokenizer::{Span, Token, TokenKind};

use crate::builder::TreeBuilder;
use crate::diagnostic::DiagnosticKind;
use crate::syntax::{Child, NodeKind, Tree};
use crate::{Parse, parse_expression};

fn tok(kind: TokenKind, start: u32, end: u32) -> Token {
    Token::new(kind, Span::new(start as usize, end as usize))
}

// ---- The tree and its builder ---------------------------------------------

/// `1 + 2`, built by hand.
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
        one_plus_two().dump(),
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
        tree.dump(),
        "(BinaryExpr (BinaryExpr (NameExpr Ident) Plus (NameExpr Ident)) Plus (NameExpr Ident))"
    );
    assert_eq!(tree.span(tree.root()), Span::new(0, 9));
}

#[test]
fn an_empty_node_is_still_a_node() {
    let mut builder = TreeBuilder::new();
    let file = builder.open(NodeKind::SourceFile);
    let error = builder.open(NodeKind::Error);
    builder.close(error);
    builder.close(file);

    let tree = builder.finish();
    assert_eq!(tree.dump(), "(SourceFile (Error))");
    assert_eq!(tree.children(tree.root()).count(), 1);
}

// ---- Parsing --------------------------------------------------------------

/// Parses, asserts the parse was clean, and renders the shape.
///
/// Parenthesized rather than dumped as kinds: what these tests are checking is
/// how §2's table groups the operands, and the operators themselves are the
/// readable way to say so.
fn shape(source: &str) -> String {
    let parse = parse_expression(source);
    assert!(
        parse.diagnostics.is_empty(),
        "{source:?} did not parse cleanly: {:?}",
        parse.diagnostics
    );
    grouped(&parse, source)
}

/// The tree written back out with each *grouping* node's extent
/// parenthesized, so a precedence mistake reads as a misplaced bracket rather
/// than as a tree diff.
///
/// Nodes that already carry their own delimiters, and leaves, are transparent
/// — bracketing `f()`'s argument list would say nothing about precedence.
/// Trivia is left in place, since these are the tokens the tree really holds.
fn grouped(parse: &Parse, source: &str) -> String {
    fn node(tree: &Tree, id: crate::NodeId, source: &str, out: &mut String) {
        let parens = !matches!(
            tree.kind(id),
            NodeKind::SourceFile
                | NodeKind::LiteralExpr
                | NodeKind::NameExpr
                | NodeKind::ParenExpr
                | NodeKind::UnitExpr
                | NodeKind::ArgList
                | NodeKind::RetType
                | NodeKind::NameType
                | NodeKind::UnitType
                | NodeKind::FnType
        );
        if parens {
            out.push('(');
        }
        for child in tree.children(id) {
            match child {
                Child::Node(child) => node(tree, child, source, out),
                Child::Token(token) => out.push_str(token.text(source)),
            }
        }
        if parens {
            out.push(')');
        }
    }

    let mut out = String::new();
    node(&parse.tree, parse.tree.root(), source, &mut out);
    out
}

/// Every rung of §2's table, each pinned against the rung below it. If this
/// passes, the table in `expr.rs` matches the table in the spec.
#[test]
fn precedence_ladder() {
    // Postfix binds tighter than unary...
    assert_eq!(shape("-a.b"), "(-(a.b))");
    assert_eq!(shape("!f(x)"), "(!(f(x)))");
    // ...unary tighter than `as`...
    assert_eq!(shape("-a as u8"), "((-a) as u8)");
    // ...`as` tighter than `*`...
    assert_eq!(shape("a as u8*b"), "((a as u8)*b)");
    // ...`*` tighter than `+`...
    assert_eq!(shape("a+b*c"), "(a+(b*c))");
    // ...`+` tighter than the shifts...
    assert_eq!(shape("a<<b+c"), "(a<<(b+c))");
    // ...shifts tighter than `&`...
    assert_eq!(shape("a&b<<c"), "(a&(b<<c))");
    // ...`&` tighter than `^`...
    assert_eq!(shape("a^b&c"), "(a^(b&c))");
    // ...`^` tighter than `|`...
    assert_eq!(shape("a|b^c"), "(a|(b^c))");
    // ...`|` tighter than comparison — §2 corrects C here, and these two
    // lines are what prove it...
    assert_eq!(shape("a&b==c"), "((a&b)==c)");
    assert_eq!(shape("a|b==c"), "((a|b)==c)");
    // ...comparison tighter than `&&`...
    assert_eq!(shape("a&&b==c"), "(a&&(b==c))");
    // ...and `&&` tighter than `||`.
    assert_eq!(shape("a||b&&c"), "(a||(b&&c))");
}

#[test]
fn binary_operators_are_left_associative() {
    assert_eq!(shape("a-b-c"), "((a-b)-c)");
    assert_eq!(shape("a/b/c"), "((a/b)/c)");
    assert_eq!(shape("a&&b&&c"), "((a&&b)&&c)");
}

#[test]
fn unary_operators_stack() {
    assert_eq!(shape("--a"), "(-(-a))");
    assert_eq!(shape("!~a"), "(!(~a))");
}

#[test]
fn postfix_chains_left() {
    assert_eq!(shape("f(x)(y)"), "((f(x))(y))");
    assert_eq!(shape("a.b.c"), "((a.b).c)");
    assert_eq!(shape("a[i][j]"), "((a[i])[j])");
    assert_eq!(shape("f(x).y[0]"), "(((f(x)).y)[0])");
}

/// A method call is its own node, not a call whose callee is a field access —
/// §2 leaves it to §11 whether `obj.method` is a value on its own, and the
/// tree must not answer that question early.
#[test]
fn a_method_call_is_not_a_call_of_a_field() {
    let parse = parse_expression("a.b(c)");
    assert!(parse.diagnostics.is_empty());
    assert_eq!(
        parse.tree.dump(),
        "(SourceFile (MethodCallExpr (NameExpr Ident) Dot Ident (ArgList LParen (NameExpr Ident) RParen)))"
    );

    let parse = parse_expression("a.b");
    assert_eq!(
        parse.tree.dump(),
        "(SourceFile (FieldExpr (NameExpr Ident) Dot Ident))"
    );
}

#[test]
fn parens_group_and_unit_is_its_own_thing() {
    // Two brackets around `a+b`: the source's, and the one `grouped` adds for
    // the node. A `ParenExpr` adds none of its own.
    assert_eq!(shape("(a+b)*c"), "(((a+b))*c)");

    let parse = parse_expression("()");
    assert!(parse.diagnostics.is_empty());
    assert_eq!(parse.tree.dump(), "(SourceFile (UnitExpr LParen RParen))");
}

#[test]
fn calls_take_argument_lists() {
    assert_eq!(shape("f()"), "(f())");
    assert_eq!(shape("f(a,b+c)"), "(f(a,(b+c)))");
    assert_eq!(shape("f(a,)"), "(f(a,))");
}

#[test]
fn casts_take_types() {
    assert_eq!(shape("a as f64"), "(a as f64)");
    assert_eq!(shape("a as ()"), "(a as ())");
    assert_eq!(
        shape("f as fn(u64, u64) -> u64"),
        "(f as fn(u64, u64) -> u64)"
    );
}

// ---- Recovery -------------------------------------------------------------

fn diagnostics(source: &str) -> Vec<DiagnosticKind> {
    parse_expression(source)
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.kind)
        .collect()
}

/// §2 makes comparison non-associative so the error names the mistake rather
/// than letting it surface later as a `bool` compared to a number.
#[test]
fn comparison_does_not_chain() {
    assert_eq!(
        diagnostics("a < b < c"),
        [DiagnosticKind::ChainedComparison]
    );
    assert_eq!(
        diagnostics("a < b == c"),
        [DiagnosticKind::ChainedComparison]
    );
    // One comparison with arithmetic on both sides is not a chain.
    assert!(diagnostics("a + b < c * d").is_empty());
    // Nor is one on each side of `&&`, which is the suggested rewrite.
    assert!(diagnostics("a < b && b < c").is_empty());
}

#[test]
fn a_missing_operand_still_parses() {
    let parse = parse_expression("1 +");
    assert_eq!(
        parse.diagnostics.iter().map(|d| d.kind).collect::<Vec<_>>(),
        [DiagnosticKind::ExpectedExpression]
    );
    assert_eq!(
        parse.tree.dump(),
        "(SourceFile (BinaryExpr (LiteralExpr Int) Whitespace Plus (Error)))"
    );
}

#[test]
fn an_unclosed_paren_is_reported_once() {
    assert_eq!(
        diagnostics("(a + b"),
        [DiagnosticKind::ExpectedClosingParen]
    );
}

/// However badly recovery goes, the tokens are all still in the tree. This is
/// the property the whole representation exists for, so it is checked on
/// garbage rather than on anything well-formed.
#[test]
fn every_byte_survives_a_bad_parse() {
    for source in [
        "1 + ",
        "( a + ",
        "a b c",
        "* * *",
        "f(,,)",
        "a . . b",
        "  // just a comment\n",
        "a as as",
        "]]]",
        "a[",
        "1 +++ 2",
    ] {
        let parse = parse_expression(source);
        assert_eq!(
            text_of(&parse, source),
            source,
            "{source:?} did not round-trip through the tree"
        );
    }
}

/// Every token in the tree, in order — which must be the source back.
fn text_of(parse: &Parse, source: &str) -> String {
    fn node(tree: &Tree, id: crate::NodeId, source: &str, out: &mut String) {
        for child in tree.children(id) {
            match child {
                Child::Node(child) => node(tree, child, source, out),
                Child::Token(token) => out.push_str(token.text(source)),
            }
        }
    }

    let mut out = String::new();
    node(&parse.tree, parse.tree.root(), source, &mut out);
    out
}
