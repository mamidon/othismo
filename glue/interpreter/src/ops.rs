//! §2's operators, applied to values.
//!
//! Separate from the executor so that the semantics are readable on their own —
//! this file is what to check against §2's table, and `exec.rs` is what to check
//! against core IR's instruction set.
//!
//! `&&` and `||` are absent, and now absent twice over: they short-circuit,
//! which makes them control flow wearing an operator's clothes (§2), so
//! elaboration lowers them to [`ir::program::Stmt::If`] and there is no
//! [`BinOp`] for either. Neither back end implements laziness.
//!
//! What §2 promises and this delivers:
//!
//! * **Overflow traps**, at the operand's own width — `255u8 + 1` traps where
//!   `255u16 + 1` does not. Every integer operation is checked.
//! * **Division truncates toward zero**, and **remainder takes the sign of the
//!   dividend** — `-7s64 / 2s64` is `-3` and `-7s64 % 2s64` is `-1`, which is
//!   what wasm's `div_s`/`rem_s` do.
//! * **Integer division and remainder by zero trap.** Float division does not:
//!   §2 also says floats follow IEEE-754, and IEEE's answer is an infinity.
//!   Trapping is the rule for the operation with no representable answer, and
//!   float division by zero has one.
//! * **Strings**: `+` concatenates, and comparison is by bytes (§1: strings are
//!   UTF-8, so byte order is code-point order).
//!
//! What is *not* here any more: the type checks. §1's "no implicit conversion"
//! and §2's "cross-type comparison does not exist" are elaboration's now, so a
//! pair of operands that disagree cannot reach this file. Where one appears to,
//! the answer is `unreachable!` rather than a message — an interpreter bug, not
//! a program's.

use std::cmp::Ordering;

use ir::program::{BinOp, UnOp};

use crate::error::TrapKind;
use crate::value::{IntTy, Value};

/// The failure carries no location — the caller has the statement's [`CstId`]
/// and adds one.
///
/// [`CstId`]: ir::program::CstId
type OpResult = Result<Value, TrapKind>;

pub(crate) fn binary(op: BinOp, left: Value, right: Value) -> OpResult {
    match (left, right) {
        (Value::Int { value: a, ty }, Value::Int { value: b, .. }) => integer(op, a, b, ty),
        (Value::Float { value: a, bits }, Value::Float { value: b, .. }) => Ok(float(op, a, b, bits)),
        (Value::Str(a), Value::Str(b)) => Ok(match op {
            // §2: `+` concatenates. Nothing else about a string is arithmetic.
            BinOp::Add => Value::string(&format!("{a}{b}")),
            _ => Value::Bool(compare(op, Some(a.as_bytes().cmp(b.as_bytes())))),
        }),
        (Value::Char(a), Value::Char(b)) => Ok(Value::Bool(compare(op, Some(a.cmp(&b))))),
        // §2 gives no meaning to `false < true`, so elaboration admits only
        // equality here and the ordering arm is unreachable.
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(compare(op, Some(a.cmp(&b))))),
        // Unit has one inhabitant, so two of them are equal (§6).
        (Value::Unit, Value::Unit) => Ok(Value::Bool(compare(op, Some(Ordering::Equal)))),
        (left, right) => unreachable!(
            "§1 has no implicit conversion, so elaboration refuses `{}` against `{}`",
            left.type_name(),
            right.type_name()
        ),
    }
}

pub(crate) fn unary(op: UnOp, operand: Value) -> OpResult {
    match (op, operand) {
        // §2 defines `-` on signed and float types only; negating an unsigned
        // value is a type error elaboration has already reported. What is left
        // is the one signed value whose negation does not fit.
        (UnOp::Neg, Value::Int { value, ty }) => checked(-value, ty, "-"),
        (UnOp::Neg, Value::Float { value, bits }) => Ok(round(-value, bits)),
        (UnOp::Not, Value::Bool(value)) => Ok(Value::Bool(!value)),
        (op, operand) => unreachable!(
            "elaboration refuses `{}` on `{}`",
            op.name(),
            operand.type_name()
        ),
    }
}

fn integer(op: BinOp, a: i128, b: i128, ty: IntTy) -> OpResult {
    let name = op.spelling();
    let value = match op {
        BinOp::Add => a.checked_add(b),
        BinOp::Sub => a.checked_sub(b),
        BinOp::Mul => a.checked_mul(b),
        // Division by zero and overflow deserve different messages, so the
        // zero case is taken first.
        BinOp::Div | BinOp::Rem if b == 0 => return Err(TrapKind::DividedByZero),
        BinOp::Div => a.checked_div(b),
        BinOp::Rem => a.checked_rem(b),
        _ => return Ok(Value::Bool(compare(op, Some(a.cmp(&b))))),
    };
    match value {
        Some(value) => checked(value, ty, name),
        // Only reachable for `i128`'s own edges, which the widths §1 has cannot
        // produce — but the answer is the same one the width check gives.
        None => Err(overflow(name, ty)),
    }
}

/// §2's overflow rule: the mathematical result, or a trap if the type cannot
/// hold it. There is no wrapping.
fn checked(value: i128, ty: IntTy, operator: &'static str) -> OpResult {
    if ty.holds(value) {
        Ok(Value::Int { value, ty })
    } else {
        Err(overflow(operator, ty))
    }
}

fn overflow(operator: &'static str, ty: IntTy) -> TrapKind {
    TrapKind::Overflow {
        operator,
        ty: ty.name(),
    }
}

fn float(op: BinOp, a: f64, b: f64, bits: u8) -> Value {
    match op {
        BinOp::Add => round(a + b, bits),
        BinOp::Sub => round(a - b, bits),
        BinOp::Mul => round(a * b, bits),
        BinOp::Div => round(a / b, bits),
        // IEEE remainder, which takes the dividend's sign exactly as the
        // integer one does.
        BinOp::Rem => round(a % b, bits),
        // `partial_cmp` gives `None` for NaN against anything, which is where
        // §2's "every ordering against NaN is false, and `NaN != NaN`" comes
        // from rather than being a rule of its own.
        _ => Value::Bool(compare(op, a.partial_cmp(&b))),
    }
}

/// An `f32` is held at `f64` width, so every operation on one rounds back —
/// which is what wasm's `f32` instructions do, and what keeps the two back ends
/// agreeing about a value neither of them can represent exactly.
fn round(value: f64, bits: u8) -> Value {
    Value::Float {
        value: if bits == 32 { value as f32 as f64 } else { value },
        bits,
    }
}

/// The six comparisons, against an ordering that may not exist.
///
/// `None` is NaN against anything. Every ordering against it is false and `!=`
/// is true, which falls out of comparing the `Option` rather than needing to be
/// written down.
fn compare(op: BinOp, order: Option<Ordering>) -> bool {
    use Ordering::*;
    match op {
        BinOp::Eq => order == Some(Equal),
        BinOp::Ne => order != Some(Equal),
        BinOp::Lt => order == Some(Less),
        BinOp::Le => matches!(order, Some(Less | Equal)),
        BinOp::Gt => order == Some(Greater),
        BinOp::Ge => matches!(order, Some(Greater | Equal)),
        _ => unreachable!("`{}` is not a comparison", op.name()),
    }
}
