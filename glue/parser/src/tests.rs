//! Tests for the tree representation, its builder, and expression parsing.

use tokenizer::{Span, Token, TokenKind};

use crate::builder::TreeBuilder;
use crate::diagnostic::DiagnosticKind;
use crate::syntax::{Child, NodeKind, Tree};
use crate::{Parse, parse};

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

/// The tree is lossless, so a node's extent starts at whatever trivia was
/// attached to its first token — a blank line above a declaration belongs to
/// the declaration. A message with a caret under it wants the first byte
/// someone wrote instead.
#[test]
fn a_significant_span_leaves_the_trivia_out() {
    let parsed = parse("let x =  1 ;");
    let tree = &parsed.tree;
    let Child::Node(let_stmt) = tree.children(tree.root()).next().unwrap() else {
        panic!("the file's first child is the `let`");
    };
    let initializer = tree
        .children(let_stmt)
        .filter_map(|child| match child {
            Child::Node(node) if tree.kind(node) == NodeKind::LiteralExpr => Some(node),
            _ => None,
        })
        .next()
        .expect("the initializer is a literal");

    // Both spaces before `1` belong to the literal; the caret should not.
    assert_eq!(tree.span(initializer), Span::new(7, 10));
    assert_eq!(tree.significant_span(initializer), Span::new(9, 10));
    // Trailing trivia is left off the same way: `;` is the `let`'s own token,
    // so the statement ends at it rather than at the space before it.
    assert_eq!(tree.significant_span(let_stmt), Span::new(0, 12));
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
/// how §expressions' table groups the operands, and the operators themselves
/// are the readable way to say so.
fn shape(source: &str) -> String {
    let parsed = parse(source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source:?} did not parse cleanly: {:?}",
        parsed.diagnostics
    );
    grouped(&parsed, source)
}

/// The tree written back out with each *grouping* node's extent
/// parenthesized, so a precedence mistake reads as a misplaced bracket rather
/// than as a tree diff.
///
/// Nodes that already carry their own delimiters, and leaves, are transparent
/// — bracketing `f()`'s argument list would say nothing about precedence.
/// Trivia is left in place, since these are the tokens the tree really holds.
fn grouped(parsed: &Parse, source: &str) -> String {
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
    node(&parsed.tree, parsed.tree.root(), source, &mut out);
    out
}

/// Every rung of §expressions' table, each pinned against the rung below it.
/// If this passes, the table in `expr.rs` matches the table in the spec.
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
    // ...`+` tighter than comparison...
    assert_eq!(shape("a+b==c"), "((a+b)==c)");
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
    assert_eq!(shape("!!a"), "(!(!a))");
}

#[test]
fn postfix_chains_left() {
    assert_eq!(shape("f(x)(y)"), "((f(x))(y))");
    assert_eq!(shape("a.b.c"), "((a.b).c)");
    assert_eq!(shape("a[i][j]"), "((a[i])[j])");
    assert_eq!(shape("f(x).y[0]"), "(((f(x)).y)[0])");
}

/// A method call is its own node, not a call whose callee is a field access —
/// §expressions leaves it to §objects whether `obj.method` is a value on its
/// own, and the tree must not answer that question early.
#[test]
fn a_method_call_is_not_a_call_of_a_field() {
    let parsed = parse("a.b(c)");
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(
        parsed.tree.dump(),
        "(SourceFile (MethodCallExpr (NameExpr Ident) Dot Ident (ArgList ParenLeft (NameExpr Ident) ParenRight)))"
    );

    let parsed = parse("a.b");
    assert_eq!(
        parsed.tree.dump(),
        "(SourceFile (FieldExpr (NameExpr Ident) Dot Ident))"
    );
}

#[test]
fn parens_group_and_unit_is_its_own_thing() {
    // Two brackets around `a+b`: the source's, and the one `grouped` adds for
    // the node. A `ParenExpr` adds none of its own.
    assert_eq!(shape("(a+b)*c"), "(((a+b))*c)");

    let parsed = parse("()");
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(
        parsed.tree.dump(),
        "(SourceFile (UnitExpr ParenLeft ParenRight))"
    );
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

// ---- Statements -----------------------------------------------------------

/// Nodes only, with tokens and trivia dropped, so a statement's shape is
/// readable without every `;` and space in the way.
fn skeleton(source: &str) -> String {
    let parsed = parse(source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source:?} did not parse cleanly: {:?}",
        parsed.diagnostics
    );
    fn node(t: &Tree, id: crate::NodeId, out: &mut String) {
        out.push('(');
        out.push_str(&format!("{:?}", t.kind(id)));
        for child in t.children(id) {
            if let Child::Node(child) = child {
                out.push(' ');
                node(t, child, out);
            }
        }
        out.push(')');
    }
    let mut out = String::new();
    node(&parsed.tree, parsed.tree.root(), &mut out);
    out
}

#[test]
fn bindings() {
    assert_eq!(
        skeleton("let n = 42;"),
        "(SourceFile (LetStmt (NamePat) (LiteralExpr)))"
    );
    assert_eq!(
        skeleton("let n: u32 = 42;"),
        "(SourceFile (LetStmt (NamePat) (NameType) (LiteralExpr)))"
    );
    // `mut` gates mutation, not rebinding (§statements) — but it is still just
    // a token.
    assert_eq!(
        skeleton("let mut count = 0;"),
        "(SourceFile (LetStmt (NamePat) (LiteralExpr)))"
    );
}

/// §statements makes assignment a statement, so `if x = 1 { … }` cannot
/// compile — the typo class is gone by construction rather than by lint.
#[test]
fn assignment_is_a_statement() {
    assert_eq!(
        skeleton("count = count + 1;"),
        "(SourceFile (AssignStmt (NameExpr) (BinaryExpr (NameExpr) (LiteralExpr))))"
    );
    // The left side is a place: a name, a field, or an index (§statements).
    assert_eq!(
        skeleton("p.x = 5;"),
        "(SourceFile (AssignStmt (FieldExpr (NameExpr)) (LiteralExpr)))"
    );
    assert_eq!(
        skeleton("a[i] = 2;"),
        "(SourceFile (AssignStmt (IndexExpr (NameExpr) (NameExpr)) (LiteralExpr)))"
    );
}

/// A file is a block (§statements): statements, then an optional trailing
/// expression with no `;` that is the file's value. That is all of goal
/// §one-language.
#[test]
fn a_file_is_a_block() {
    assert_eq!(
        skeleton("let x = 2;\nx * 21"),
        "(SourceFile (LetStmt (NamePat) (LiteralExpr)) (BinaryExpr (NameExpr) (LiteralExpr)))"
    );
    // A bare expression is a whole program.
    assert_eq!(skeleton("42"), "(SourceFile (LiteralExpr))");
    // And so is nothing at all.
    assert_eq!(skeleton(""), "(SourceFile)");
}

#[test]
fn blocks_are_expressions() {
    assert_eq!(
        skeleton("let y = { let t = f(); t * 2 };"),
        "(SourceFile (LetStmt (NamePat) (BlockExpr \
         (LetStmt (NamePat) (CallExpr (NameExpr) (ArgList))) \
         (BinaryExpr (NameExpr) (LiteralExpr)))))"
    );
    // The trailing `;` discards, so the block's value is unit — same tree
    // shape, one more ExprStmt.
    assert_eq!(
        skeleton("{ f(); }"),
        "(SourceFile (BlockExpr (ExprStmt (CallExpr (NameExpr) (ArgList)))))"
    );
}

/// §control: braces mandatory, condition unparenthesized, and `else if` is
/// `else` followed by another `if` rather than a keyword of its own.
#[test]
fn conditionals() {
    assert_eq!(
        skeleton("let x = if ok { 42 } else { 0 };"),
        "(SourceFile (LetStmt (NamePat) (IfExpr (NameExpr) (BlockExpr (LiteralExpr)) \
         (BlockExpr (LiteralExpr)))))"
    );
    assert_eq!(
        skeleton("if a { 1 } else if b { 2 } else { 3 }"),
        "(SourceFile (IfExpr (NameExpr) (BlockExpr (LiteralExpr)) \
         (IfExpr (NameExpr) (BlockExpr (LiteralExpr)) (BlockExpr (LiteralExpr)))))"
    );
}

/// A block-shaped expression standing as a statement needs no `;`.
#[test]
fn block_like_statements_need_no_semicolon() {
    assert_eq!(
        skeleton("if a { 1 } else { 2 }\nlet x = 1;"),
        "(SourceFile (ExprStmt (IfExpr (NameExpr) (BlockExpr (LiteralExpr)) \
         (BlockExpr (LiteralExpr)))) (LetStmt (NamePat) (LiteralExpr)))"
    );
    // A plain expression in the same position does need one.
    assert_eq!(
        diagnostics("f() let x = 1;"),
        [DiagnosticKind::ExpectedSemicolon]
    );
}

#[test]
fn loops_and_jumps() {
    assert_eq!(
        skeleton("while total < 10 { total = add(total, 1); }"),
        "(SourceFile (WhileStmt (BinaryExpr (NameExpr) (LiteralExpr)) \
         (BlockExpr (AssignStmt (NameExpr) (CallExpr (NameExpr) \
         (ArgList (NameExpr) (LiteralExpr)))))))"
    );
    assert_eq!(
        skeleton("while c { break; continue; }"),
        "(SourceFile (WhileStmt (NameExpr) (BlockExpr (BreakStmt) (ContinueStmt))))"
    );
    assert_eq!(skeleton("return;"), "(SourceFile (ReturnStmt))");
    assert_eq!(
        skeleton("return value;"),
        "(SourceFile (ReturnStmt (NameExpr)))"
    );
}

#[test]
fn declarations() {
    assert_eq!(
        skeleton("fn add(a: u64, b: u64) -> u64 { a + b }"),
        "(SourceFile (FnDecl (ParamList (Param (NameType)) (Param (NameType))) \
         (RetType (NameType)) (BlockExpr (BinaryExpr (NameExpr) (NameExpr)))))"
    );
    // §functions: the return type is omitted when it is unit.
    assert_eq!(
        skeleton("fn log(message: Str) { }"),
        "(SourceFile (FnDecl (ParamList (Param (NameType))) (BlockExpr)))"
    );
    // §functions: `mut` on a parameter is the parameter's, not the type's.
    assert_eq!(
        skeleton("fn advance(c: mut Counter, by: u64) { }"),
        "(SourceFile (FnDecl (ParamList (Param (NameType)) (Param (NameType))) (BlockExpr)))"
    );
    assert_eq!(
        skeleton("struct Point {\n  x: s64,\n  y: s64,\n}"),
        "(SourceFile (StructDecl (FieldDeclList (FieldDecl (NameType)) (FieldDecl (NameType)))))"
    );
    assert_eq!(
        skeleton("type InstanceId = u64;"),
        "(SourceFile (TypeAliasDecl (NameType)))"
    );
}

/// §functions: a `fn` may be declared inside a block, and captures nothing.
#[test]
fn declarations_nest() {
    assert_eq!(
        skeleton("fn outer() { fn inner() { } inner() }"),
        "(SourceFile (FnDecl (ParamList) (BlockExpr (FnDecl (ParamList) (BlockExpr)) \
         (CallExpr (NameExpr) (ArgList)))))"
    );
}

#[test]
fn struct_literals() {
    assert_eq!(
        skeleton("let origin = Point { x: 0, y: 0 };"),
        "(SourceFile (LetStmt (NamePat) (StructLitExpr (NameExpr) \
         (FieldInitList (FieldInit (LiteralExpr)) (FieldInit (LiteralExpr))))))"
    );
}

/// A struct literal is banned in a condition, so the brace there is the body.
/// Without the ban `if flag { … }` would read `flag { … }` as a literal and
/// then find no body at all.
#[test]
fn a_condition_takes_its_brace_as_the_body() {
    assert_eq!(
        skeleton("if flag { 1 } else { 2 }"),
        "(SourceFile (IfExpr (NameExpr) (BlockExpr (LiteralExpr)) (BlockExpr (LiteralExpr))))"
    );
    assert_eq!(
        skeleton("while ready { step(); }"),
        "(SourceFile (WhileStmt (NameExpr) (BlockExpr (ExprStmt (CallExpr (NameExpr) (ArgList))))))"
    );
    // The ban does not reach inside brackets, where a brace is unambiguous.
    assert_eq!(
        skeleton("if f(Point { x: 0 }) { 1 }"),
        "(SourceFile (IfExpr (CallExpr (NameExpr) (ArgList (StructLitExpr (NameExpr) \
         (FieldInitList (FieldInit (LiteralExpr)))))) (BlockExpr (LiteralExpr))))"
    );
    assert_eq!(
        skeleton("if (Point { x: 0 }) == p { 1 }"),
        "(SourceFile (IfExpr (BinaryExpr (ParenExpr (StructLitExpr (NameExpr) \
         (FieldInitList (FieldInit (LiteralExpr))))) (NameExpr)) (BlockExpr (LiteralExpr))))"
    );
}

/// `(x) -> body`. The parameter list is spelled exactly like a parenthesized
/// expression, so only the `->` after the `)` tells them apart.
#[test]
fn lambdas() {
    assert_eq!(
        skeleton("let inc = (x: u64) -> x + 1;"),
        "(SourceFile (LetStmt (NamePat) (LambdaExpr (LambdaParamList (LambdaParam (NameType))) \
         (BinaryExpr (NameExpr) (LiteralExpr)))))"
    );
    // §functions: parameter types come from context, unlike a `fn`.
    assert_eq!(
        skeleton("let inc = (x) -> x + 1;"),
        "(SourceFile (LetStmt (NamePat) (LambdaExpr (LambdaParamList (LambdaParam)) \
         (BinaryExpr (NameExpr) (LiteralExpr)))))"
    );
    assert_eq!(
        skeleton("let go = () -> work();"),
        "(SourceFile (LetStmt (NamePat) (LambdaExpr (LambdaParamList) \
         (CallExpr (NameExpr) (ArgList)))))"
    );
    assert_eq!(
        skeleton("let add = (x, y) -> x + y;"),
        "(SourceFile (LetStmt (NamePat) (LambdaExpr \
         (LambdaParamList (LambdaParam) (LambdaParam)) \
         (BinaryExpr (NameExpr) (NameExpr)))))"
    );
}

/// The same text without a trailing `->` is grouping, or unit.
#[test]
fn a_paren_is_not_a_lambda_without_an_arrow() {
    assert_eq!(skeleton("(a)"), "(SourceFile (ParenExpr (NameExpr)))");
    assert_eq!(skeleton("()"), "(SourceFile (UnitExpr))");
    assert_eq!(
        skeleton("(a + b) * c"),
        "(SourceFile (BinaryExpr (ParenExpr (BinaryExpr (NameExpr) (NameExpr))) (NameExpr)))"
    );
    // `||` stays the operator; there is no lambda spelling that collides.
    assert_eq!(
        skeleton("a || b"),
        "(SourceFile (BinaryExpr (NameExpr) (NameExpr)))"
    );
    // The lookahead has to cross nested brackets to find the `->`.
    assert_eq!(
        skeleton("let f = (x: fn(u64) -> u64) -> x;"),
        "(SourceFile (LetStmt (NamePat) (LambdaExpr \
         (LambdaParamList (LambdaParam (FnType (NameType) (RetType (NameType))))) (NameExpr))))"
    );
}

/// Every file in `examples/`, so they can serve as a regression suite rather
/// than as documentation nobody runs.
const EXAMPLES: [(&str, &str); 5] = [
    ("hello.glue", include_str!("../../examples/hello.glue")),
    (
        "literals.glue",
        include_str!("../../examples/literals.glue"),
    ),
    (
        "expressions.glue",
        include_str!("../../examples/expressions.glue"),
    ),
    (
        "statements.glue",
        include_str!("../../examples/statements.glue"),
    ),
    (
        "declarations.glue",
        include_str!("../../examples/declarations.glue"),
    ),
];

/// Every example lexes clean, parses clean, and comes back out byte for byte.
#[test]
fn the_examples_parse_cleanly() {
    for (name, source) in EXAMPLES {
        let lexed = tokenizer::tokenize(source);
        assert!(
            lexed.diagnostics.is_empty(),
            "{name} did not lex cleanly: {:?}",
            lexed.diagnostics
        );

        let parsed = parse(source);
        assert!(
            parsed.diagnostics.is_empty(),
            "{name} did not parse cleanly: {:?}",
            parsed.diagnostics
        );
        assert_eq!(
            text_of(&parsed, source),
            source,
            "{name} did not round-trip through the tree"
        );
    }
}

/// The examples cover the whole grammar.
///
/// This is what makes "every expression form and every statement" a fact
/// rather than a claim: adding a `NodeKind` without an example that produces
/// it fails here, and the examples stay honest as the language grows.
#[test]
fn every_kind_has_an_example() {
    let mut seen = std::collections::HashSet::new();
    for (_, source) in EXAMPLES {
        let parsed = parse(source);
        let tree = &parsed.tree;
        let mut stack = vec![tree.root()];
        while let Some(node) = stack.pop() {
            seen.insert(tree.kind(node));
            stack.extend(tree.children(node).filter_map(|child| match child {
                Child::Node(child) => Some(child),
                Child::Token(_) => None,
            }));
        }
    }

    let missing: Vec<_> = NodeKind::ALL
        .iter()
        // The examples all parse cleanly, so an error node is exactly what
        // should never turn up in one.
        .filter(|kind| **kind != NodeKind::Error && !seen.contains(kind))
        .collect();
    assert!(missing.is_empty(), "no example produces {missing:?}");
    assert!(
        !seen.contains(&NodeKind::Error),
        "an example produced an error node"
    );
}

// ---- Recovery -------------------------------------------------------------

fn diagnostics(source: &str) -> Vec<DiagnosticKind> {
    parse(source)
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.kind)
        .collect()
}

/// §expressions makes comparison non-associative so the error names the
/// mistake rather than letting it surface later as a `bool` compared to a
/// number.
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
    let parsed = parse("1 +");
    assert_eq!(
        parsed
            .diagnostics
            .iter()
            .map(|d| d.kind)
            .collect::<Vec<_>>(),
        [DiagnosticKind::ExpectedExpression]
    );
    assert_eq!(
        parsed.tree.dump(),
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
        // Expressions.
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
        // Statements.
        "let",
        "let x",
        "let x =",
        "let x = ;",
        "let = 1;",
        "fn",
        "fn f",
        "fn f(",
        "fn f() {",
        "fn f(a) { }",
        "fn f(a:) { }",
        "struct",
        "struct S {",
        "struct S { x }",
        "type",
        "type T =",
        "while",
        "while c",
        "while { }",
        "if",
        "if c",
        "if c { } else",
        "return",
        "break",
        "{",
        "}",
        ";;;",
        "let x = 1 let y = 2;",
        "(x",
        "(x)",
        "(x) ->",
        "() ->",
        "P { x: }",
        "P { : 1 }",
        "/// dangling\n",
    ] {
        let parsed = parse(source);
        assert_eq!(
            text_of(&parsed, source),
            source,
            "{source:?} did not round-trip through the tree"
        );
    }
}

/// Every prefix of a real program parses.
///
/// Half-typed input is the *normal* case for an editor, not the exceptional
/// one, and a prefix is exactly what half-typed looks like. Each one must
/// terminate, produce a tree, and give the source back — the three things
/// totality and losslessness actually promise.
#[test]
fn every_truncation_of_the_example_parses() {
    let source = include_str!("../../examples/hello.glue");
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let parsed = parse(prefix);
        assert_eq!(
            text_of(&parsed, prefix),
            prefix,
            "the first {end} bytes did not round-trip"
        );
    }
}

/// Every token in the tree, in order — which must be the source back.
fn text_of(parsed: &Parse, source: &str) -> String {
    fn node(tree: &Tree, id: crate::NodeId, source: &str, out: &mut String) {
        for child in tree.children(id) {
            match child {
                Child::Node(child) => node(tree, child, source, out),
                Child::Token(token) => out.push_str(token.text(source)),
            }
        }
    }

    let mut out = String::new();
    node(&parsed.tree, parsed.tree.root(), source, &mut out);
    out
}
