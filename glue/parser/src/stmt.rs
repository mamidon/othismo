//! Statements and declarations.
//!
//! §statements collapses Lox's declaration/statement split: `fn`, `struct`,
//! and `type` are statements like any other, because mandatory braces
//! (§control) already make `if x { let y = 1; }` the only spelling of the
//! thing the split existed to reject. So there is one production here, not
//! two.
//!
//! A file is a block (§statements), so [`statements`] serves both — the only
//! difference is whether it stops at `}` or at the end of input. Both may end
//! in a trailing expression with no `;`, which is that block's value
//! (§expressions).

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
    match cursor.peek() {
        TokenKind::Let => let_stmt(cursor),
        TokenKind::Fn => fn_decl(cursor),
        TokenKind::Struct => struct_decl(cursor),
        TokenKind::Type => type_alias(cursor),
        TokenKind::While => while_stmt(cursor),
        TokenKind::Break => jump(cursor, NodeKind::BreakStmt),
        TokenKind::Continue => jump(cursor, NodeKind::ContinueStmt),
        TokenKind::Return => return_stmt(cursor),
        TokenKind::Semicolon => {
            // §statements has no empty statement, so this is always a mistake
            // — but a harmless one, and the `;` is consumed so it can't be
            // re-reported.
            cursor.error(DiagnosticKind::StraySemicolon);
            let mark = cursor.open(NodeKind::Error);
            cursor.bump();
            cursor.close(mark);
        }
        _ => expr_or_assign(cursor, terminator),
    }
}

/// An expression statement, an assignment, or the block's trailing value.
///
/// Which of the three it is isn't known until the expression has been read, so
/// all three start the same way and the node opens retroactively.
fn expr_or_assign(cursor: &mut Cursor, terminator: TokenKind) {
    let parsed = expr(cursor);
    // Captured now, because `open_before` consumes `parsed`.
    let block_like = expr::is_block_like(parsed.kind());

    // §statements' compound forms — `+=` and the rest — are not in the core,
    // so `=` is the only thing that makes this an assignment.
    if cursor.at(TokenKind::Equals) {
        // §statements: the left side is a *place* — a name, a field, or an
        // index. That it actually is one is checked later, so the message can
        // name what was assigned to rather than just refusing to parse.
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
        return; // The trailing expression, and so the block's value (§expressions).
    }

    // §statements' `exprStmt → expression ";"` read literally would demand
    // `if c { … };`. A block-shaped expression is already delimited, so it
    // stands as a statement on its own.
    if !block_like {
        cursor.error(DiagnosticKind::ExpectedSemicolon);
    }
    let mark = cursor.open_before(parsed, NodeKind::ExprStmt);
    cursor.close(mark);
}

// ---- Bindings --------------------------------------------------------------

/// `let mut? pattern (: Type)? = expr ;` (§statements).
fn let_stmt(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::LetStmt);
    cursor.bump(); // `let`
    cursor.eat(TokenKind::Mut);
    pattern(cursor);
    if cursor.eat(TokenKind::Colon) {
        ty(cursor);
    }
    // §statements: always required. There is no declare-then-assign, and so no
    // definite-assignment analysis to specify or implement.
    cursor.expect(TokenKind::Equals, DiagnosticKind::ExpectedInitializer);
    expr(cursor);
    cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
    cursor.close(mark);
}

/// The whole of patterns today: a plain name (§statements). §unions adds the
/// rest, and a `let` whose second child is already a pattern node absorbs that
/// unchanged.
fn pattern(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::NamePat);
    cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
    cursor.close(mark);
}

// ---- Declarations ----------------------------------------------------------

/// `fn name(a: T, b: mut U) -> R { … }` (§functions).
fn fn_decl(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::FnDecl);
    cursor.bump(); // `fn`
    cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);

    let params = cursor.open(NodeKind::ParamList);
    cursor.expect(TokenKind::ParenLeft, DiagnosticKind::ExpectedOpeningParen);
    while !cursor.at(TokenKind::ParenRight) && !cursor.at_eof() {
        let param = cursor.open(NodeKind::Param);
        // §comptime: the argument must be known at compile time. It belongs to
        // the parameter, like `mut` below — but it is written *before* the
        // name rather than after the colon, because it constrains the argument
        // rather than modifying the type.
        cursor.eat(TokenKind::Comptime);
        cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
        // §functions: parameter types are required — signatures are annotated,
        // bodies inferred, and that boundary is what lets a reader know what a
        // function means without reading it.
        cursor.expect(TokenKind::Colon, DiagnosticKind::ExpectedParameterType);
        // `mut` belongs to the parameter, not the type (§functions), so it
        // lives on `Param` even though it is written where a type modifier
        // would be.
        cursor.eat(TokenKind::Mut);
        ty(cursor);
        cursor.close(param);
        if !cursor.eat(TokenKind::Comma) {
            break;
        }
    }
    cursor.expect(TokenKind::ParenRight, DiagnosticKind::ExpectedClosingParen);
    cursor.close(params);

    // §functions: omitted when the return type is unit.
    if cursor.at(TokenKind::Arrow) {
        let ret = cursor.open(NodeKind::RetType);
        cursor.bump();
        ty(cursor);
        cursor.close(ret);
    }

    block(cursor);
    cursor.close(mark);
}

/// `struct Name { x: T, y: U, }` (§types).
fn struct_decl(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::StructDecl);
    cursor.bump(); // `struct`
    cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);

    let fields = cursor.open(NodeKind::FieldDeclList);
    cursor.expect(TokenKind::BraceLeft, DiagnosticKind::ExpectedOpeningBrace);
    while !cursor.at(TokenKind::BraceRight) && !cursor.at_eof() {
        let field = cursor.open(NodeKind::FieldDecl);
        cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
        // §types: field types are required; there is no inference across a
        // declaration boundary.
        cursor.expect(TokenKind::Colon, DiagnosticKind::ExpectedColon);
        ty(cursor);
        cursor.close(field);
        if !cursor.eat(TokenKind::Comma) {
            break;
        }
    }
    cursor.expect(TokenKind::BraceRight, DiagnosticKind::ExpectedClosingBrace);
    cursor.close(fields);
    cursor.close(mark);
}

/// `type Name = T ;` — a second name for one type, not a new one (§types).
fn type_alias(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::TypeAliasDecl);
    cursor.bump(); // `type`
    cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
    cursor.expect(TokenKind::Equals, DiagnosticKind::ExpectedInitializer);
    ty(cursor);
    cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
    cursor.close(mark);
}

// ---- Control flow ----------------------------------------------------------

/// `while c { … }` — the only loop (§control), and a statement, so its value
/// is unit.
fn while_stmt(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::WhileStmt);
    cursor.bump(); // `while`
    condition(cursor);
    block(cursor);
    cursor.close(mark);
}

/// `break ;` and `continue ;` — unlabelled, applying to the innermost loop
/// (§control).
fn jump(cursor: &mut Cursor, kind: NodeKind) {
    let mark = cursor.open(kind);
    cursor.bump();
    cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
    cursor.close(mark);
}

/// `return ;` or `return expr ;` — for *early* exit (§control). A well-shaped
/// function often has none, since a body is a block and ends in its value.
fn return_stmt(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::ReturnStmt);
    cursor.bump(); // `return`
    if !cursor.at(TokenKind::Semicolon) {
        expr(cursor);
    }
    cursor.expect(TokenKind::Semicolon, DiagnosticKind::ExpectedSemicolon);
    cursor.close(mark);
}
