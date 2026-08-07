//! Statements and declarations.
//!
//! §3 collapses Lox's declaration/statement split: `fn`, `struct`, and `type`
//! are statements like any other, because mandatory braces (§4) already make
//! `if x { let y = 1; }` the only spelling of the thing the split existed to
//! reject. So there is one production here, not two.
//!
//! A file is a block (§3), so [`statements`] serves both — the only difference
//! is whether it stops at `}` or at the end of input. Both may end in a
//! trailing expression with no `;`, which is that block's value (§2).

use tokenizer::TokenKind;

use crate::cursor::Cursor;
use crate::diagnostic::DiagnosticKind;
use crate::expr::{self, block, condition, expr, ty};
use crate::syntax::NodeKind;

/// Statements up to `terminator`, then stop without consuming it.
pub fn statements(cursor: &mut Cursor, terminator: TokenKind) {
    while !cursor.at(terminator) && !cursor.at_eof() {
        let before = cursor.position();
        statement(cursor, terminator);

        // Every rule below either consumes something or reports why it
        // couldn't. If one managed neither, the loop would spin, so a token is
        // consumed here to guarantee the parse terminates on any input.
        if cursor.position() == before {
            cursor.error(DiagnosticKind::UnexpectedInput);
            let mark = cursor.open(NodeKind::Error);
            cursor.bump();
            cursor.close(mark);
        }
    }
}

fn statement(cursor: &mut Cursor, terminator: TokenKind) {
    // §1: a doc comment attaches to what follows it, so the declaration has to
    // open before them. They aren't trivia, which is why they're counted here
    // rather than flushed like whitespace.
    let docs = doc_comments(cursor);
    let leading = cursor.nth(docs);

    if docs > 0 && (leading == terminator || leading == TokenKind::Eof) {
        // §1 promises a warning for a doc comment attached to nothing.
        cursor.error(DiagnosticKind::DanglingDocComment);
        bump_docs(cursor, docs);
        return;
    }

    match leading {
        TokenKind::Let => let_stmt(cursor, docs),
        TokenKind::Fn => fn_decl(cursor, docs),
        TokenKind::Struct => struct_decl(cursor, docs),
        TokenKind::Type => type_alias(cursor, docs),
        TokenKind::While => while_stmt(cursor, docs),
        TokenKind::Break => jump(cursor, docs, NodeKind::BreakStmt),
        TokenKind::Continue => jump(cursor, docs, NodeKind::ContinueStmt),
        TokenKind::Return => return_stmt(cursor, docs),
        TokenKind::Semicolon => {
            // §3 has no empty statement, so this is always a mistake — but a
            // harmless one, and the `;` is consumed so it can't be re-reported.
            cursor.error(DiagnosticKind::StraySemicolon);
            let mark = cursor.open(NodeKind::Error);
            bump_docs(cursor, docs);
            cursor.bump();
            cursor.close(mark);
        }
        _ => expr_or_assign(cursor, docs, terminator),
    }
}

/// An expression statement, an assignment, or the block's trailing value.
///
/// Which of the three it is isn't known until the expression has been read, so
/// all three start the same way and the node opens retroactively.
fn expr_or_assign(cursor: &mut Cursor, docs: usize, terminator: TokenKind) {
    // A doc comment on a plain expression is attached to nothing worth naming.
    if docs > 0 {
        cursor.error(DiagnosticKind::DanglingDocComment);
        bump_docs(cursor, docs);
    }

    let parsed = expr(cursor);
    // Captured now, because `open_before` consumes `parsed`.
    let block_like = expr::is_block_like(parsed.kind());

    if is_assignment_operator(cursor.peek()) {
        // §3: the left side is a *place* — a name, a field, or an index. That
        // it actually is one is checked later, so the message can name what
        // was assigned to rather than just refusing to parse.
        let mark = cursor.open_before(parsed, NodeKind::AssignStmt);
        cursor.bump();
        expr(cursor);
        cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
        cursor.close(mark);
        return;
    }

    if cursor.at(TokenKind::Semicolon) {
        let mark = cursor.open_before(parsed, NodeKind::ExprStmt);
        cursor.bump();
        cursor.close(mark);
        return;
    }

    // No `;`. Either this is the block's value, or a `;` is missing — except
    // after a block-shaped expression, which needs none.
    let at_end = cursor.at(terminator) || cursor.at_eof();
    if at_end {
        return; // The trailing expression, and so the block's value (§2).
    }

    // §3's `exprStmt → expression ";"` read literally would demand
    // `if c { … };`. A block-shaped expression is already delimited, so it
    // stands as a statement on its own.
    if !block_like {
        cursor.error(DiagnosticKind::ExpectedSemicolon);
    }
    let mark = cursor.open_before(parsed, NodeKind::ExprStmt);
    cursor.close(mark);
}

fn is_assignment_operator(kind: TokenKind) -> bool {
    use TokenKind::*;
    // §3's compound forms. Each is exactly `a = a op b` with the place
    // evaluated once, so they are spelled out rather than derived.
    matches!(
        kind,
        Eq | PlusEq
            | MinusEq
            | StarEq
            | SlashEq
            | PercentEq
            | AmpEq
            | PipeEq
            | CaretEq
            | ShlEq
            | ShrEq
    )
}

// ---- Bindings --------------------------------------------------------------

/// `let mut? pattern (: Type)? = expr ;` (§3).
fn let_stmt(cursor: &mut Cursor, docs: usize) {
    let mark = cursor.open(NodeKind::LetStmt);
    bump_docs(cursor, docs);
    cursor.bump(); // `let`
    cursor.eat(TokenKind::Mut);
    pattern(cursor);
    if cursor.eat(TokenKind::Colon) {
        ty(cursor);
    }
    // §3: always required. There is no declare-then-assign, and so no
    // definite-assignment analysis to specify or implement.
    cursor.expect(TokenKind::Eq, DiagnosticKind::ExpectedInitializer);
    expr(cursor);
    cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
    cursor.close(mark);
}

/// The whole of patterns today: a plain name (§3). §7 adds the rest, and a
/// `let` whose second child is already a pattern node absorbs that unchanged.
fn pattern(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::NamePat);
    cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
    cursor.close(mark);
}

// ---- Declarations ----------------------------------------------------------

/// `fn name(a: T, b: mut U) -> R { … }` (§5).
fn fn_decl(cursor: &mut Cursor, docs: usize) {
    let mark = cursor.open(NodeKind::FnDecl);
    bump_docs(cursor, docs);
    cursor.bump(); // `fn`
    cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);

    let params = cursor.open(NodeKind::ParamList);
    cursor.expect(TokenKind::LParen, DiagnosticKind::ExpectedOpeningParen);
    while !cursor.at(TokenKind::RParen) && !cursor.at_eof() {
        let param = cursor.open(NodeKind::Param);
        cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
        // §5: parameter types are required — signatures are annotated, bodies
        // inferred, and that boundary is what lets a reader know what a
        // function means without reading it.
        cursor.expect(TokenKind::Colon, DiagnosticKind::ExpectedParameterType);
        // `mut` belongs to the parameter, not the type (§5), so it lives on
        // `Param` even though it is written where a type modifier would be.
        cursor.eat(TokenKind::Mut);
        ty(cursor);
        cursor.close(param);
        if !cursor.eat(TokenKind::Comma) {
            break;
        }
    }
    cursor.expect(TokenKind::RParen, DiagnosticKind::ExpectedClosingParen);
    cursor.close(params);

    // §5: omitted when the return type is unit.
    if cursor.at(TokenKind::Arrow) {
        let ret = cursor.open(NodeKind::RetType);
        cursor.bump();
        ty(cursor);
        cursor.close(ret);
    }

    block(cursor);
    cursor.close(mark);
}

/// `struct Name { x: T, y: U, }` (§6).
fn struct_decl(cursor: &mut Cursor, docs: usize) {
    let mark = cursor.open(NodeKind::StructDecl);
    bump_docs(cursor, docs);
    cursor.bump(); // `struct`
    cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);

    let fields = cursor.open(NodeKind::FieldDeclList);
    cursor.expect(TokenKind::LBrace, DiagnosticKind::ExpectedOpeningBrace);
    while !cursor.at(TokenKind::RBrace) && !cursor.at_eof() {
        let field = cursor.open(NodeKind::FieldDecl);
        bump_docs(cursor, doc_comments(cursor));
        cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
        // §6: field types are required; there is no inference across a
        // declaration boundary.
        cursor.expect(TokenKind::Colon, DiagnosticKind::ExpectedColon);
        ty(cursor);
        cursor.close(field);
        if !cursor.eat(TokenKind::Comma) {
            break;
        }
    }
    cursor.expect(TokenKind::RBrace, DiagnosticKind::ExpectedClosingBrace);
    cursor.close(fields);
    cursor.close(mark);
}

/// `type Name = T ;` — a second name for one type, not a new one (§6).
fn type_alias(cursor: &mut Cursor, docs: usize) {
    let mark = cursor.open(NodeKind::TypeAliasDecl);
    bump_docs(cursor, docs);
    cursor.bump(); // `type`
    cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
    cursor.expect(TokenKind::Eq, DiagnosticKind::ExpectedInitializer);
    ty(cursor);
    cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
    cursor.close(mark);
}

// ---- Control flow ----------------------------------------------------------

/// `while c { … }` — the only loop (§4), and a statement, so its value is unit.
fn while_stmt(cursor: &mut Cursor, docs: usize) {
    let mark = cursor.open(NodeKind::WhileStmt);
    bump_docs(cursor, docs);
    cursor.bump(); // `while`
    condition(cursor);
    block(cursor);
    cursor.close(mark);
}

/// `break ;` and `continue ;` — unlabelled, applying to the innermost loop (§4).
fn jump(cursor: &mut Cursor, docs: usize, kind: NodeKind) {
    let mark = cursor.open(kind);
    bump_docs(cursor, docs);
    cursor.bump();
    cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
    cursor.close(mark);
}

/// `return ;` or `return expr ;` — for *early* exit (§4). A well-shaped
/// function often has none, since a body is a block and ends in its value.
fn return_stmt(cursor: &mut Cursor, docs: usize) {
    let mark = cursor.open(NodeKind::ReturnStmt);
    bump_docs(cursor, docs);
    cursor.bump(); // `return`
    if !cursor.at(TokenKind::Semicolon) {
        expr(cursor);
    }
    cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
    cursor.close(mark);
}

// ---- Doc comments ----------------------------------------------------------

fn doc_comments(cursor: &Cursor) -> usize {
    let mut count = 0;
    while cursor.nth(count) == TokenKind::DocComment {
        count += 1;
    }
    count
}

fn bump_docs(cursor: &mut Cursor, count: usize) {
    for _ in 0..count {
        cursor.bump();
    }
}
