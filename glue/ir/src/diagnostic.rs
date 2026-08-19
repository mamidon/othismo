//! What elaboration complains about.
//!
//! Shaped like [`parser::Diagnostic`] and [`tokenizer::Diagnostic`] — a kind
//! plus a span — but with kinds that carry data, unlike theirs. A lexical or
//! grammatical problem can be named without saying anything about the program;
//! a type error cannot, and "expected `u64`, found `Str`" is the whole value of
//! the message.

use tokenizer::{Severity, Span};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(kind: DiagnosticKind, span: Span) -> Diagnostic {
        Diagnostic { kind, span }
    }

    pub fn severity(&self) -> Severity {
        Severity::Error
    }

    pub fn message(&self) -> String {
        self.kind.message()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DiagnosticKind {
    // ---- Names ------------------------------------------------------------
    UnknownName(String),
    UnknownType(String),
    UnknownField {
        ty: String,
        field: String,
    },
    /// §5: a nested `fn` captures nothing. The name exists — it is just not
    /// reachable from here.
    FnCapturesNothing(String),
    NotAType(String),
    NotAValue(String),

    // ---- Types ------------------------------------------------------------
    TypeMismatch {
        expected: String,
        found: String,
    },
    /// §2 has no truthiness: a condition is a `bool` or it is an error.
    ConditionNotBool(String),
    /// §1: `u64 + s64` and `u32 + u64` are both errors. There is no promotion
    /// lattice.
    MixedOperands {
        op: String,
        left: String,
        right: String,
    },
    OperatorNotDefined {
        op: String,
        ty: String,
    },
    /// §2: negating an unsigned value has no representable result but zero, so
    /// it is refused at compile time rather than trapped at run time.
    NegateUnsigned(String),
    NotCallable(String),
    WrongArity {
        expected: usize,
        found: usize,
    },
    NotAStruct(String),
    MissingField(String),
    DuplicateField(String),
    /// §5: a lambda's types come from context, and there wasn't any.
    CannotInferLambda,
    CannotCast {
        from: String,
        to: String,
    },

    // ---- Constants --------------------------------------------------------
    /// §1: pinning by context failed, and pinning by sign would not fit either.
    ConstantOutOfRange {
        value: String,
        ty: String,
    },
    /// §2: constant expressions are checked at compile time rather than
    /// trapping, so this is where overflow lands when every operand is
    /// constant.
    ConstantOverflow,
    ConstantDivisionByZero,
    /// §1: integer and float constants do not mix.
    MixedConstantKinds,

    // ---- Statements -------------------------------------------------------
    /// §3: the left side of an assignment is a place — a name, a field, or an
    /// index.
    NotAPlace,
    /// §6: field mutability follows the binding, so a non-`mut` binding permits
    /// assigning no field.
    AssignToNonMut(String),
    /// §4: unlabelled, and applying to the innermost enclosing loop — of which
    /// there is none here.
    JumpOutsideLoop(&'static str),

    // ---- Not yet ----------------------------------------------------------
    /// A construct the parser accepts and elaboration has no answer for,
    /// because the section that owns it is unstarted.
    Unsupported(&'static str),
}

impl DiagnosticKind {
    pub fn message(&self) -> String {
        use DiagnosticKind::*;
        match self {
            UnknownName(name) => format!("no binding named `{name}` is in scope"),
            UnknownType(name) => format!("no type named `{name}` is in scope"),
            UnknownField { ty, field } => format!("`{ty}` has no field `{field}`"),
            FnCapturesNothing(name) => format!(
                "`{name}` is a local of an enclosing function, and a `fn` captures nothing — \
                 use a lambda"
            ),
            NotAType(name) => format!("`{name}` is a value, not a type"),
            NotAValue(name) => format!("`{name}` is a type, not a value"),
            TypeMismatch { expected, found } => format!("expected `{expected}`, found `{found}`"),
            ConditionNotBool(found) => format!(
                "a condition must be `bool`, and this is `{found}` — there is no truthiness"
            ),
            MixedOperands { op, left, right } => format!(
                "`{op}` needs both operands to have the same type, and these are `{left}` and \
                 `{right}` — conversions are explicit"
            ),
            OperatorNotDefined { op, ty } => format!("`{op}` is not defined on `{ty}`"),
            NegateUnsigned(ty) => {
                format!("`{ty}` is unsigned, so there is no value for `-` to produce")
            }
            NotCallable(ty) => format!("`{ty}` is not a function"),
            WrongArity { expected, found } => {
                format!("this function takes {expected} argument(s), and {found} were given")
            }
            NotAStruct(name) => format!("`{name}` is not a struct"),
            MissingField(name) => format!("field `{name}` is missing"),
            DuplicateField(name) => format!("field `{name}` is given twice"),
            CannotInferLambda => "a lambda's types come from context, and there is none here — \
                 annotate its parameters, or give the binding a type"
                .to_string(),
            CannotCast { from, to } => format!("there is no conversion from `{from}` to `{to}`"),
            ConstantOutOfRange { value, ty } => {
                format!("the constant {value} does not fit in `{ty}`")
            }
            ConstantOverflow => {
                "this constant expression overflows — constants are checked at compile time"
                    .to_string()
            }
            ConstantDivisionByZero => "this constant expression divides by zero".to_string(),
            MixedConstantKinds => {
                "integer and float constants do not mix — write a suffix or a cast".to_string()
            }
            NotAPlace => "only a name, a field, or an index can be assigned to".to_string(),
            AssignToNonMut(name) => format!(
                "`{name}` is not `mut`, so its fields cannot be assigned — write `let mut {name}`"
            ),
            JumpOutsideLoop(word) => format!("`{word}` is only meaningful inside a loop"),
            Unsupported(what) => format!("{what} is not supported yet"),
        }
    }
}
