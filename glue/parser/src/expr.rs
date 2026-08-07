//! Expressions, by precedence climbing.
//!
//! §2's precedence table is a ladder of twelve rungs, and writing it as twelve
//! mutually recursive functions would mean twelve near-identical bodies and a
//! stack frame per rung for every operand. It is written here as binding
//! powers instead: one loop, one table, and the table is the only thing to
//! read when checking it against the spec.
//!
//! Binding power runs the other way from §2's level numbers — level 1 binds
//! tightest, so it gets the *largest* power. A left-associative operator takes
//! `(2n, 2n + 1)`, so its right operand refuses to swallow another operator at
//! the same level and the tree leans left.

use tokenizer::TokenKind;

use crate::builder::Closed;
use crate::cursor::Cursor;
use crate::diagnostic::DiagnosticKind;
use crate::syntax::NodeKind;

/// §2's ladder, tightest first. Names here are the table's row labels; the
/// numbers are derived from them so that the two can't drift.
mod level {
    pub const POSTFIX: u8 = 12; // f(…)  a[…]  .field  .method(…)
    pub const UNARY: u8 = 11; //   -  !  ~
    pub const AS: u8 = 10;
    pub const PRODUCT: u8 = 9; //  *  /  %
    pub const SUM: u8 = 8; //      +  -
    pub const SHIFT: u8 = 7; //    <<  >>
    pub const BIT_AND: u8 = 6; //  &
    pub const BIT_XOR: u8 = 5; //  ^
    pub const BIT_OR: u8 = 4; //   |
    pub const COMPARE: u8 = 3; //  ==  !=  <  <=  >  >=   (non-associative)
    pub const AND: u8 = 2; //      &&
    pub const OR: u8 = 1; //       ||
}

/// A left-associative operator's `(left, right)` binding power.
const fn left_assoc(level: u8) -> (u8, u8) {
    (level * 2, level * 2 + 1)
}

/// Whether an expression carries its own braces.
///
/// §3's `exprStmt → expression ";"` read literally would require
/// `if c { … };`. These forms are already delimited, so the `;` is optional
/// after them and their appearance mid-block is unambiguous.
pub fn is_block_like(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::BlockExpr | NodeKind::IfExpr)
}

/// Parses one expression, and everything binding tighter than `min_bp`.
///
/// Always produces a node — [`NodeKind::Error`] when there is no expression to
/// be had — so a caller never has to decide what to do about nothing.
pub fn expr(cursor: &mut Cursor) -> Closed {
    expr_bp(cursor, 0)
}

fn expr_bp(cursor: &mut Cursor, min_bp: u8) -> Closed {
    let mut lhs = if is_unary_operator(cursor.peek()) {
        let mark = cursor.open(NodeKind::UnaryExpr);
        cursor.bump();
        expr_bp(cursor, level::UNARY * 2);
        cursor.close(mark)
    } else {
        primary(cursor)
    };

    // §2 makes comparison non-associative. The loop still runs twice on
    // `a < b < c` — recovering is better than stopping — but the second turn
    // is reported rather than quietly building `(a < b) < c`.
    let mut previous_was_comparison = false;

    loop {
        let operator = cursor.peek();

        if operator == TokenKind::As {
            if level::AS * 2 < min_bp {
                break;
            }
            let mark = cursor.open_before(lhs, NodeKind::CastExpr);
            cursor.bump();
            ty(cursor);
            lhs = cursor.close(mark);
            continue;
        }

        // `Point { x: 1 }` (§6). Only after a bare name, and only where a
        // brace can't be a block instead — see `Cursor::no_struct_literal`.
        if operator == TokenKind::LBrace
            && lhs.kind() == NodeKind::NameExpr
            && cursor.struct_literals_allowed()
        {
            let mark = cursor.open_before(lhs, NodeKind::StructLitExpr);
            field_init_list(cursor);
            lhs = cursor.close(mark);
            continue;
        }

        if let Some(kind) = postfix_operator(cursor, operator) {
            if level::POSTFIX * 2 < min_bp {
                break;
            }
            lhs = postfix(cursor, lhs, kind);
            continue;
        }

        let Some((left_bp, right_bp)) = binary_operator(operator) else {
            break;
        };
        if left_bp < min_bp {
            break;
        }

        let comparison = left_bp == level::COMPARE * 2;
        if comparison && previous_was_comparison {
            cursor.error(DiagnosticKind::ChainedComparison);
        }
        previous_was_comparison = comparison;

        let mark = cursor.open_before(lhs, NodeKind::BinaryExpr);
        cursor.bump();
        expr_bp(cursor, right_bp);
        lhs = cursor.close(mark);
    }

    lhs
}

/// §2's prefix operators. No `+`, which would be a no-op with a spelling.
fn is_unary_operator(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Minus | TokenKind::Bang | TokenKind::Tilde)
}

fn binary_operator(kind: TokenKind) -> Option<(u8, u8)> {
    use TokenKind::*;
    Some(left_assoc(match kind {
        Star | Slash | Percent => level::PRODUCT,
        Plus | Minus => level::SUM,
        Shl | Shr => level::SHIFT,
        Amp => level::BIT_AND,
        Caret => level::BIT_XOR,
        Pipe => level::BIT_OR,
        EqEq | BangEq | Lt | Le | Gt | Ge => level::COMPARE,
        AmpAmp => level::AND,
        PipePipe => level::OR,
        _ => return None,
    }))
}

/// Which postfix form follows, if any. `.` needs two tokens of lookahead to
/// tell a method call from a field access, which is the only place in the
/// expression grammar that does.
fn postfix_operator(cursor: &Cursor, kind: TokenKind) -> Option<NodeKind> {
    Some(match kind {
        TokenKind::LParen => NodeKind::CallExpr,
        TokenKind::LBracket => NodeKind::IndexExpr,
        TokenKind::Dot => {
            if cursor.nth(1) == TokenKind::Ident && cursor.nth(2) == TokenKind::LParen {
                NodeKind::MethodCallExpr
            } else {
                NodeKind::FieldExpr
            }
        }
        _ => return None,
    })
}

fn postfix(cursor: &mut Cursor, lhs: Closed, kind: NodeKind) -> Closed {
    let mark = cursor.open_before(lhs, kind);
    match kind {
        NodeKind::CallExpr => arg_list(cursor),
        NodeKind::IndexExpr => {
            cursor.bump(); // `[`
            let allowed = cursor.set_struct_literals_allowed(true);
            expr(cursor);
            cursor.set_struct_literals_allowed(allowed);
            cursor.expect(TokenKind::RBracket, DiagnosticKind::ExpectedClosingBracket);
        }
        NodeKind::FieldExpr => {
            cursor.bump(); // `.`
            cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedFieldName);
        }
        NodeKind::MethodCallExpr => {
            cursor.bump(); // `.`
            cursor.bump(); // name
            arg_list(cursor);
        }
        _ => unreachable!("postfix_operator produces only the four kinds above"),
    }
    cursor.close(mark)
}

fn arg_list(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::ArgList);
    cursor.bump(); // `(`
    // Inside brackets a `{` can only be a struct literal, so whatever
    // restriction a surrounding condition imposed does not reach here.
    let allowed = cursor.set_struct_literals_allowed(true);
    while !cursor.at(TokenKind::RParen) && !cursor.at_eof() {
        expr(cursor);
        // Progress is the comma's job: an argument that parsed nothing leaves
        // the cursor where it was, so without a separator to consume there is
        // nothing left to try.
        if !cursor.eat(TokenKind::Comma) {
            break;
        }
    }
    cursor.set_struct_literals_allowed(allowed);
    cursor.expect(TokenKind::RParen, DiagnosticKind::ExpectedClosingParen);
    cursor.close(mark);
}

/// `{ x: 1, y: 2 }` — the body of a struct literal (§6).
fn field_init_list(cursor: &mut Cursor) {
    let mark = cursor.open(NodeKind::FieldInitList);
    cursor.bump(); // `{`
    let allowed = cursor.set_struct_literals_allowed(true);
    while !cursor.at(TokenKind::RBrace) && !cursor.at_eof() {
        let field = cursor.open(NodeKind::FieldInit);
        cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedFieldName);
        cursor.expect(TokenKind::Colon, DiagnosticKind::ExpectedColon);
        expr(cursor);
        cursor.close(field);
        if !cursor.eat(TokenKind::Comma) {
            break;
        }
    }
    cursor.set_struct_literals_allowed(allowed);
    cursor.expect(TokenKind::RBrace, DiagnosticKind::ExpectedClosingBrace);
    cursor.close(mark);
}

/// `{ … }` — an expression (§2), so a function body, an `if` arm, and a `let`
/// initializer are all the same node.
pub fn block(cursor: &mut Cursor) -> Closed {
    let mark = cursor.open(NodeKind::BlockExpr);
    cursor.expect(TokenKind::LBrace, DiagnosticKind::ExpectedOpeningBrace);
    // A brace is unambiguous once we're inside one.
    let allowed = cursor.set_struct_literals_allowed(true);
    crate::stmt::statements(cursor, TokenKind::RBrace);
    cursor.set_struct_literals_allowed(allowed);
    cursor.expect(TokenKind::RBrace, DiagnosticKind::ExpectedClosingBrace);
    cursor.close(mark)
}

/// The condition of an `if` or `while`. Braces are mandatory and the condition
/// is unparenthesized (§4), which is what makes the struct-literal ban
/// necessary.
pub fn condition(cursor: &mut Cursor) {
    let allowed = cursor.set_struct_literals_allowed(false);
    expr(cursor);
    cursor.set_struct_literals_allowed(allowed);
}

/// `if c { … } else if d { … } else { … }` — an expression (§2).
///
/// `else if` is `else` followed by another `if` rather than a keyword of its
/// own (§4), so the chain nests instead of flattening.
fn if_expr(cursor: &mut Cursor) -> Closed {
    let mark = cursor.open(NodeKind::IfExpr);
    cursor.bump(); // `if`
    condition(cursor);
    block(cursor);
    if cursor.eat(TokenKind::Else) {
        if cursor.at(TokenKind::If) {
            if_expr(cursor);
        } else {
            block(cursor);
        }
    }
    cursor.close(mark)
}

/// `|x| x + 1`, or `|| work()` (§5).
///
/// Parameter types are optional here, unlike a `fn`: a declaration is read by
/// others, a lambda is read in place.
fn lambda(cursor: &mut Cursor) -> Closed {
    let mark = cursor.open(NodeKind::LambdaExpr);
    let params = cursor.open(NodeKind::LambdaParamList);
    if cursor.at(TokenKind::PipePipe) {
        // §5: in operand position `||` opens a lambda with no parameters. It
        // is the same token as the operator, and only the position tells them
        // apart — the wart Rust has, which has not proved to be a problem.
        cursor.bump();
    } else {
        cursor.bump(); // `|`
        while !cursor.at(TokenKind::Pipe) && !cursor.at_eof() {
            let param = cursor.open(NodeKind::LambdaParam);
            cursor.expect(TokenKind::Ident, DiagnosticKind::ExpectedName);
            if cursor.eat(TokenKind::Colon) {
                ty(cursor);
            }
            cursor.close(param);
            if !cursor.eat(TokenKind::Comma) {
                break;
            }
        }
        cursor.expect(TokenKind::Pipe, DiagnosticKind::ExpectedClosingPipe);
    }
    cursor.close(params);
    expr(cursor);
    cursor.close(mark)
}

/// The operand at the bottom of the ladder.
fn primary(cursor: &mut Cursor) -> Closed {
    let kind = cursor.peek();

    if kind.is_literal() {
        let mark = cursor.open(NodeKind::LiteralExpr);
        cursor.bump();
        return cursor.close(mark);
    }

    match kind {
        TokenKind::Ident => {
            let mark = cursor.open(NodeKind::NameExpr);
            cursor.bump();
            cursor.close(mark)
        }
        TokenKind::LParen => {
            // `()` is the unit value, not an empty grouping (§6).
            let unit = cursor.nth(1) == TokenKind::RParen;
            let mark = cursor.open(if unit {
                NodeKind::UnitExpr
            } else {
                NodeKind::ParenExpr
            });
            cursor.bump(); // `(`
            if !unit {
                let allowed = cursor.set_struct_literals_allowed(true);
                expr(cursor);
                cursor.set_struct_literals_allowed(allowed);
            }
            cursor.expect(TokenKind::RParen, DiagnosticKind::ExpectedClosingParen);
            cursor.close(mark)
        }
        TokenKind::LBrace => block(cursor),
        TokenKind::If => if_expr(cursor),
        TokenKind::Pipe | TokenKind::PipePipe => lambda(cursor),
        // Nothing is consumed. The caller's loop then finds no operator and
        // stops, or finds one and consumes it — either way the parse advances,
        // and whatever is really here is swept up by the enclosing construct.
        _ => cursor.error_node(DiagnosticKind::ExpectedExpression),
    }
}

/// Types, to the extent `as` needs them.
///
/// §6 and §8 own this properly — generic arguments, and whatever §8's
/// collections spell. What's here is what can be written today.
pub fn ty(cursor: &mut Cursor) -> Closed {
    match cursor.peek() {
        TokenKind::Ident => {
            let mark = cursor.open(NodeKind::NameType);
            cursor.bump();
            cursor.close(mark)
        }
        TokenKind::LParen if cursor.nth(1) == TokenKind::RParen => {
            let mark = cursor.open(NodeKind::UnitType);
            cursor.bump();
            cursor.bump();
            cursor.close(mark)
        }
        TokenKind::Fn => {
            let mark = cursor.open(NodeKind::FnType);
            cursor.bump(); // `fn`
            cursor.expect(TokenKind::LParen, DiagnosticKind::ExpectedClosingParen);
            while !cursor.at(TokenKind::RParen) && !cursor.at_eof() {
                ty(cursor);
                if !cursor.eat(TokenKind::Comma) {
                    break;
                }
            }
            cursor.expect(TokenKind::RParen, DiagnosticKind::ExpectedClosingParen);
            if cursor.at(TokenKind::Arrow) {
                let ret = cursor.open(NodeKind::RetType);
                cursor.bump();
                ty(cursor);
                cursor.close(ret);
            }
            cursor.close(mark)
        }
        _ => cursor.error_node(DiagnosticKind::ExpectedType),
    }
}
