//! What a Glue program evaluates to.
//!
//! §1's six kinds of literal. There is no `nil`: §1 doesn't have one, so
//! neither does this, and [`Value::Unit`] is a real value with one inhabitant
//! rather than an absence (§5).
//!
//! **A value carries its type.** §1's numeric tower is real now — elaboration
//! pins every constant and every slot to a width before this crate sees it — so
//! `255u8 + 1` has to trap where `255u16 + 1` does not, and the check needs the
//! width at hand. Core IR keeps types in slots (invariant 1), so the width
//! could equally be looked up from the slot an operand names; carrying it in
//! the value instead keeps [`crate::ops`] a function of values alone, which is
//! what makes that file checkable against §2's table on its own.
//!
//! The alternative reading — the one this crate held while it walked the
//! concrete syntax tree — was that every integer is an `i64` and every float an
//! `f64`, with a literal's suffix read and discarded. §10's arrival is what
//! ends that.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use ir::program::FuncId;

/// One of §1's integer types: `u8`…`u64` and `s8`…`s64`.
///
/// `s` rather than `i`, matching wasm's instruction suffixes and the IR's
/// [`ir::types::TypeDef::Int`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IntTy {
    pub signed: bool,
    pub bits: u8,
}

impl IntTy {
    pub const U64: IntTy = IntTy {
        signed: false,
        bits: 64,
    };
    pub const S64: IntTy = IntTy {
        signed: true,
        bits: 64,
    };

    pub fn new(signed: bool, bits: u8) -> IntTy {
        IntTy { signed, bits }
    }

    pub fn min(self) -> i128 {
        if self.signed {
            -(1i128 << (self.bits - 1))
        } else {
            0
        }
    }

    pub fn max(self) -> i128 {
        if self.signed {
            (1i128 << (self.bits - 1)) - 1
        } else {
            (1i128 << self.bits) - 1
        }
    }

    /// Whether a mathematical value is representable in this type — §2's
    /// overflow check, and the only thing standing between a `u8` and wasm's
    /// silent wrapping.
    pub fn holds(self, value: i128) -> bool {
        value >= self.min() && value <= self.max()
    }

    pub fn name(self) -> String {
        format!("{}{}", if self.signed { "s" } else { "u" }, self.bits)
    }
}

/// A runtime value.
///
/// An integer is held as its mathematical value at `i128`, wide enough for
/// every one of §1's types, with [`IntTy`] saying which of them it belongs to.
/// That is deliberately not a machine representation: §2 asks for a *checked*
/// answer to every operation, and checking is easier from the value than from
/// its bits.
///
/// `Rc<str>` for strings so that copying one is a refcount bump. Nothing
/// mutates a string in place — §2's `+` produces a new one — so shared
/// ownership needs no interior mutability to go with it.
#[derive(Clone, Debug)]
pub enum Value {
    Unit,
    Bool(bool),
    Int {
        value: i128,
        ty: IntTy,
    },
    /// `bits` is 32 or 64. An `f32` is held at `f64` width and rounded back
    /// after every operation, which is what wasm's `f32` instructions do.
    Float {
        value: f64,
        bits: u8,
    },
    Char(char),
    Str(Rc<str>),
    /// A `fn` or a lambda (§5). Every function value is a closure, and a plain
    /// `fn` has an empty environment — one representation, because the
    /// difference between the two forms was spent during elaboration.
    Closure(Rc<Closure>),
    /// A one-word box holding a value, which no Glue program can name.
    ///
    /// Lowering introduces one for a binding that is both captured and
    /// assigned, so that §5's promise — a captured binding outlives its frame,
    /// and everyone holding it sees the same writes — has somewhere to be
    /// true. An uncaptured binding is a slot and an unassigned capture is a
    /// copy; neither costs this.
    Cell(Rc<RefCell<Value>>),
}

/// A function value: which function, and the environment it closed over.
///
/// The captures are positional, filling the slots directly after the
/// parameters — `[params][captures][locals]` is the calling convention, so
/// entering a closure is two copies into the front of a frame and no lookup.
///
/// `name` is here only so a value can be echoed. It is the name elaboration
/// gave the function, which for a lambda is its parent's plus `.λn`.
#[derive(Debug)]
pub struct Closure {
    pub(crate) func: FuncId,
    pub(crate) captures: Vec<Value>,
    pub(crate) name: Rc<str>,
}

/// Structural, as §2 asks.
///
/// Two integers of different types never meet — §1 has no promotion lattice and
/// elaboration refuses the comparison — so the type is compared as well, and
/// disagreement is inequality rather than a panic.
impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Unit, Value::Unit) => true,
            (Value::Bool(left), Value::Bool(right)) => left == right,
            (
                Value::Int { value: left, ty: a },
                Value::Int {
                    value: right,
                    ty: b,
                },
            ) => a == b && left == right,
            // IEEE-754 (§2), so `NaN != NaN`.
            (
                Value::Float {
                    value: left,
                    bits: a,
                },
                Value::Float {
                    value: right,
                    bits: b,
                },
            ) => a == b && left == right,
            (Value::Char(left), Value::Char(right)) => left == right,
            (Value::Str(left), Value::Str(right)) => left == right,
            // §2 defines equality on values and identity on instance
            // references, and says nothing about functions — elaboration
            // refuses to compare two of them. This impl still has to answer,
            // and answers identity, which is the only thing that could be
            // meant. A cell is storage rather than a value, so the same holds.
            (Value::Closure(left), Value::Closure(right)) => Rc::ptr_eq(left, right),
            (Value::Cell(left), Value::Cell(right)) => Rc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl Value {
    pub fn int(value: i128, ty: IntTy) -> Value {
        Value::Int { value, ty }
    }

    pub fn u64(value: u64) -> Value {
        Value::int(value as i128, IntTy::U64)
    }

    pub fn s64(value: i64) -> Value {
        Value::int(value as i128, IntTy::S64)
    }

    pub fn f64(value: f64) -> Value {
        Value::Float { value, bits: 64 }
    }

    pub fn f32(value: f32) -> Value {
        Value::Float {
            value: value as f64,
            bits: 32,
        }
    }

    pub fn string(text: &str) -> Value {
        Value::Str(Rc::from(text))
    }

    /// How to name this value's type in a message.
    ///
    /// Exact, unlike the vague "an integer" this crate used to give: there is a
    /// type checker in front of it now, and a value knows which of §1's types
    /// it belongs to.
    pub fn type_name(&self) -> String {
        match self {
            Value::Unit => "()".to_string(),
            Value::Bool(_) => "bool".to_string(),
            Value::Int { ty, .. } => ty.name(),
            Value::Float { bits, .. } => format!("f{bits}"),
            Value::Char(_) => "char".to_string(),
            Value::Str(_) => "Str".to_string(),
            Value::Closure(_) => "a function".to_string(),
            Value::Cell(inner) => format!("(cell {})", inner.borrow().type_name()),
        }
    }
}

/// How the interpreter *echoes* a value, which is not quite how a program would
/// print one.
///
/// Strings and characters come back quoted and escaped. A bare `hello` on the
/// terminal is ambiguous between the string and a name that wasn't evaluated,
/// and the whole point of echoing the final value is to say what it is.
///
/// The type is not shown. A dump names types because it is read against the
/// IR (`ir::dump`); an echoed value is read against the program that produced
/// it, where `42` is the answer and `42u64` is the answer plus a reminder.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Unit => f.write_str("()"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Int { value, .. } => write!(f, "{value}"),
            // A float prints with a decimal point even when it lands on a whole
            // number, so `2.0` doesn't read back as an integer. Past the point
            // where every `f64` is whole, the extra `.0` is noise.
            Value::Float { value, .. } => {
                if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e16 {
                    write!(f, "{value:.1}")
                } else {
                    write!(f, "{value}")
                }
            }
            Value::Char(value) => write!(f, "'{}'", value.escape_debug()),
            Value::Str(value) => write!(f, "\"{}\"", value.escape_debug()),
            // Nothing useful to show: a body is a block of IR, and the
            // environment it closed over is not the reader's business. The
            // name is.
            Value::Closure(closure) => write!(f, "<fn {}>", closure.name),
            // A program cannot hold one, so echoing it means the executor put
            // a cell somewhere a value belongs. Shown rather than hidden, so
            // that the bug is visible.
            Value::Cell(inner) => write!(f, "(cell {})", inner.borrow()),
        }
    }
}
