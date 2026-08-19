//! What a Glue program evaluates to.
//!
//! Six variants, because §1 has six kinds of literal and this stage of the
//! interpreter adds nothing that isn't written down in the source. There is no
//! `nil`: §1 doesn't have one, so neither does this, and [`Value::Unit`] is a
//! real value with one inhabitant rather than an absence (§5).
//!
//! **Integers are `i64` and floats are `f64`, full stop.** §1's numeric tower —
//! `u8` through `s64`, `f32` and `f64` — is a *typing* distinction, and there is
//! no type checker yet, so pinning a literal to a width here would be inventing
//! an answer that §10 owns. The consequence is honest and worth knowing: a
//! suffix is read and discarded, and `255u8 + 1` is `256` rather than the
//! overflow trap §2 promises. Everything else about §2's arithmetic — trapping
//! overflow, truncating division, a remainder that takes the dividend's sign —
//! holds at `i64` width.

use std::fmt;
use std::rc::Rc;

/// A runtime value.
///
/// `Rc<str>` for strings so that binding one to a second name is a refcount
/// bump rather than a copy. Nothing mutates a string in place — §2's `+`
/// produces a new one — so shared ownership needs no interior mutability to go
/// with it.
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Char(char),
    Str(Rc<str>),
}

impl Value {
    pub fn string(text: &str) -> Value {
        Value::Str(Rc::from(text))
    }

    /// How to name this value's type in a message.
    ///
    /// Deliberately vague — "an integer", not `u64`. The interpreter doesn't
    /// know which of §1's integer types a value belongs to, and a message that
    /// named one would be guessing.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Unit => "unit",
            Value::Bool(_) => "a boolean",
            Value::Int(_) => "an integer",
            Value::Float(_) => "a float",
            Value::Char(_) => "a character",
            Value::Str(_) => "a string",
        }
    }
}

/// How the interpreter *echoes* a value, which is not quite how a program
/// would print one.
///
/// Strings and characters come back quoted and escaped. A bare `hello` on the
/// terminal is ambiguous between the string and a name that wasn't evaluated,
/// and the whole point of echoing the final value is to say what it is.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Unit => f.write_str("()"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Int(value) => write!(f, "{value}"),
            // A float prints with a decimal point even when it lands on a whole
            // number, so `2.0` doesn't read back as an integer. Past the point
            // where every `f64` is whole, the extra `.0` is noise.
            Value::Float(value) => {
                if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e16 {
                    write!(f, "{value:.1}")
                } else {
                    write!(f, "{value}")
                }
            }
            Value::Char(value) => write!(f, "'{}'", value.escape_debug()),
            Value::Str(value) => write!(f, "\"{}\"", value.escape_debug()),
        }
    }
}
