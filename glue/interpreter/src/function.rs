//! What a function value is.
//!
//! One type for both forms §5 has, differing in what they capture rather than
//! in what they are — a `fn` is a declaration others read, a lambda is an
//! argument read in place, and by the time either is a value that difference
//! has already been spent.
//!
//! The body is a [`NodeId`] rather than anything compiled: this is a tree
//! walker, so calling a function means walking the same subtree again. It is
//! also the part most obviously waiting for the IL.
//!
//! `pub` only because [`crate::Value`] holds one; every field is crate-private,
//! so from outside a function value is opaque and stays that way when the IL
//! replaces what is in here.

use std::fmt;
use std::rc::Rc;

use parser::NodeId;

use crate::env::Frame;

#[derive(Clone, Debug)]
pub(crate) struct Param {
    pub(crate) name: Rc<str>,
    /// §5: `mut` means the function may mutate the caller's value, and the
    /// caller must pass a `mut` binding. It also makes the parameter itself a
    /// mutable binding, since writing to it is the whole point.
    pub(crate) mutable: bool,
}

pub struct Function {
    /// `None` for a lambda, which is anonymous by construction.
    pub(crate) name: Option<Rc<str>>,
    pub(crate) params: Vec<Param>,
    /// A `BlockExpr` for a `fn`, any expression for a lambda (§5).
    pub(crate) body: NodeId,
    /// The frames the body runs against. §5: a `fn` captures nothing, which is
    /// spelled here as keeping the frames and raising `barrier` over all of
    /// them — see [`crate::env`].
    pub(crate) captured: Vec<Frame>,
    pub(crate) barrier: usize,
}

impl Function {
    /// How to name this function in a message.
    pub(crate) fn describe(&self) -> String {
        match &self.name {
            Some(name) => format!("`{name}`"),
            None => "this lambda".to_string(),
        }
    }
}

/// Hand-written, and shallow on purpose: a frame can hold a function that
/// captured that same frame, so deriving this would recurse forever the first
/// time anything printed a value.
impl fmt::Debug for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "fn {name}/{}", self.params.len()),
            None => write!(f, "lambda/{}", self.params.len()),
        }
    }
}
