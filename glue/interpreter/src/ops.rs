//! §2's operators, applied to values.
//!
//! Separate from the tree walk so that the semantics are readable on their own
//! — this file is what to check against §2's table, and `eval.rs` is what to
//! check against the grammar.
//!
//! `&&` and `||` are absent, deliberately. They short-circuit, which makes them
//! control flow wearing an operator's clothes (§2), and control flow needs the
//! unevaluated right operand rather than its value. They live in `eval.rs` with
//! the other things that decide what to evaluate.
//!
//! What §2 promises and this delivers:
//!
//! * **Overflow traps.** Every integer operation is checked.
//! * **Division truncates toward zero**, and **remainder takes the sign of the
//!   dividend** — `-7 / 2` is `-3` and `-7 % 2` is `-1`, which is what Rust's
//!   `i64` operators already do and what wasm's `div_s`/`rem_s` do.
//! * **Integer division and remainder by zero trap.** Float division does not:
//!   §2 also says floats follow IEEE-754, and IEEE's answer is an infinity.
//!   Trapping is the rule for the operation that has no representable answer,
//!   and float division by zero has one.
//! * **No implicit conversion** (§1). `1 + 1.5` is an error, not a widening.
//! * **Cross-type comparison does not exist** (§2). `1 == "1"` is an error, not
//!   `false`.
//! * **Strings**: `+` concatenates, and comparison is by bytes (§1: strings are
//!   UTF-8, so byte order is code-point order).

use tokenizer::TokenKind;

use crate::error::RuntimeErrorKind;
use crate::value::Value;

/// The failure carries no span — the caller has the node and adds one.
type OpResult = Result<Value, RuntimeErrorKind>;

/// Every binary operator but `&&` and `||`.
pub(crate) fn binary(operator: TokenKind, left: Value, right: Value) -> OpResult {
    use TokenKind::*;
    match operator {
        Plus | Minus | Star | Slash | Percent => arithmetic(operator, left, right),
        EqualTo | NotEqualTo => equality(operator, left, right),
        LessThan | LessThanOrEqualTo | GreaterThan | GreaterThanOrEqualTo => {
            ordering(operator, left, right)
        }
        _ => unreachable!("the parser builds a BinaryExpr for no other operator"),
    }
}

pub(crate) fn unary(operator: TokenKind, operand: Value) -> OpResult {
    let name = spelling(operator);
    match (operator, &operand) {
        // §2 defines `-` on signed and float types only, and negating an
        // unsigned value is a type error. Which of §1's integer types a value
        // has is §10's to decide, so every integer is negatable here and the
        // rule arrives with the type checker.
        (TokenKind::Minus, Value::Int(value)) => value
            .checked_neg()
            .map(Value::Int)
            .ok_or(RuntimeErrorKind::Overflow(name)),
        (TokenKind::Minus, Value::Float(value)) => Ok(Value::Float(-value)),
        (TokenKind::Bang, Value::Bool(value)) => Ok(Value::Bool(!value)),
        _ => Err(RuntimeErrorKind::UnaryTypeMismatch {
            operator: name,
            operand: operand.type_name(),
        }),
    }
}

fn arithmetic(operator: TokenKind, left: Value, right: Value) -> OpResult {
    use TokenKind::*;
    let name = spelling(operator);
    match (&left, &right) {
        (Value::Int(a), Value::Int(b)) => {
            let (a, b) = (*a, *b);
            match operator {
                Plus => a.checked_add(b),
                Minus => a.checked_sub(b),
                Star => a.checked_mul(b),
                // `checked_div` and `checked_rem` fold division by zero in with
                // overflow, and the two deserve different messages.
                Slash | Percent if b == 0 => return Err(RuntimeErrorKind::DividedByZero),
                Slash => a.checked_div(b),
                Percent => a.checked_rem(b),
                _ => unreachable!("arithmetic is called for no other operator"),
            }
            .map(Value::Int)
            .ok_or(RuntimeErrorKind::Overflow(name))
        }

        (Value::Float(a), Value::Float(b)) => {
            let (a, b) = (*a, *b);
            Ok(Value::Float(match operator {
                Plus => a + b,
                Minus => a - b,
                Star => a * b,
                Slash => a / b,
                // IEEE remainder, which takes the dividend's sign exactly as
                // the integer one does.
                Percent => a % b,
                _ => unreachable!("arithmetic is called for no other operator"),
            }))
        }

        // §2: `+` concatenates strings. No other operator applies to one.
        (Value::Str(a), Value::Str(b)) if operator == Plus => Ok(Value::string(&format!("{a}{b}"))),

        _ => Err(mismatch(name, &left, &right)),
    }
}

fn equality(operator: TokenKind, left: Value, right: Value) -> OpResult {
    if !same_type(&left, &right) {
        return Err(mismatch(spelling(operator), &left, &right));
    }
    // Structural, and IEEE for floats — so `NaN != NaN`, which is what Rust's
    // own `PartialEq` on `f64` gives.
    let equal = left == right;
    Ok(Value::Bool(match operator {
        TokenKind::EqualTo => equal,
        _ => !equal,
    }))
}

fn ordering(operator: TokenKind, left: Value, right: Value) -> OpResult {
    use std::cmp::Ordering::*;

    // `partial_cmp` returning `None` is NaN against anything, and §2 says all
    // four comparisons against NaN are `false`.
    let order = match (&left, &right) {
        (Value::Int(a), Value::Int(b)) => a.partial_cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::Char(a), Value::Char(b)) => a.partial_cmp(b),
        // Byte order, which for UTF-8 is code-point order.
        (Value::Str(a), Value::Str(b)) => a.as_bytes().partial_cmp(b.as_bytes()),
        // Booleans and unit are equatable but not ordered: §2 gives no meaning
        // to `false < true`, and inventing one here would be inventing language.
        _ => return Err(mismatch(spelling(operator), &left, &right)),
    };

    Ok(Value::Bool(match operator {
        TokenKind::LessThan => order == Some(Less),
        TokenKind::LessThanOrEqualTo => matches!(order, Some(Less | Equal)),
        TokenKind::GreaterThan => order == Some(Greater),
        TokenKind::GreaterThanOrEqualTo => matches!(order, Some(Greater | Equal)),
        _ => unreachable!("ordering is called for no other operator"),
    }))
}

/// Whether two values are of the same type — which for now is whether they are
/// the same variant, since §1's numeric tower isn't represented yet.
fn same_type(left: &Value, right: &Value) -> bool {
    use Value::*;
    matches!(
        (left, right),
        (Unit, Unit)
            | (Bool(_), Bool(_))
            | (Int(_), Int(_))
            | (Float(_), Float(_))
            | (Char(_), Char(_))
            | (Str(_), Str(_))
    )
}

fn mismatch(operator: &'static str, left: &Value, right: &Value) -> RuntimeErrorKind {
    RuntimeErrorKind::BinaryTypeMismatch {
        operator,
        left: left.type_name(),
        right: right.type_name(),
    }
}

/// Every operator that reaches here has a fixed spelling, so the fallback is
/// unreachable rather than a real name.
fn spelling(operator: TokenKind) -> &'static str {
    operator.spelling().unwrap_or("that operator")
}
