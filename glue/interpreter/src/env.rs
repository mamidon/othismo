//! Bindings, the frames they live in, and what a function can see of them.
//!
//! A chain of frames, innermost last. §12 is still empty, so the two rules that
//! already exist elsewhere are the two implemented here:
//!
//! * **Shadowing is allowed, including in the same scope** (§3). `let input =
//!   parse(input);` is the natural way to write a narrowing pipeline without
//!   inventing `input2`. That falls out of `declare` overwriting rather than
//!   refusing.
//! * **`mut` gates mutation, not rebinding** (§3). Assigning to a binding
//!   declared without `mut` is an error; declaring it again with `let` is not.
//!
//! # Why a frame is shared and mutable
//!
//! §5 makes lambdas **capture by reference**: a captured binding outlives the
//! frame that created it, and mutation through one holder is visible to every
//! other. `Rc<RefCell<…>>` per frame is that, exactly — a closure keeps its
//! frames alive by holding them, and writes through them because they are the
//! same frames the creator has.
//!
//! (It also means a function stored in the frame it captured is a reference
//! cycle, which is a leak. Collecting those is §15's problem and nobody's
//! today: a program that runs to completion and exits leaks nothing anyone can
//! observe.)
//!
//! # The barrier
//!
//! §5: a `fn` **captures nothing** — it is an ordinary function that happens to
//! be private to its block, and every `fn` compiles to a plain wasm function
//! with no environment. But it must still be able to call other functions,
//! including itself.
//!
//! So a call doesn't hand the callee an empty chain; it hands it the chain from
//! the declaration and a *barrier*, an index below which only function values
//! are visible. A name found under the barrier is reported rather than silently
//! resolved further out, because "a `fn` captures nothing — use a lambda" is
//! the thing worth saying. A lambda inherits its creator's barrier instead of
//! raising a new one, so a lambda written inside a `fn` cannot see past what
//! the `fn` could.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::value::Value;

/// A name, what it holds, and whether it may be assigned to.
#[derive(Clone, Debug)]
pub(crate) struct Binding {
    value: Value,
    mutable: bool,
}

/// One scope's worth of bindings, shared with every closure that captured it.
pub(crate) type Frame = Rc<RefCell<HashMap<String, Binding>>>;

/// Why a name didn't resolve. The caller turns these into a
/// [`crate::RuntimeError`], because it is the one that knows the span.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LookupError {
    Unknown,
    /// The name is bound outside the enclosing `fn`, and isn't a function —
    /// so the `fn` would have had to capture it, and §5 says it doesn't.
    NotCaptured,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AssignError {
    Unknown,
    NotCaptured,
    Immutable,
}

/// The frames and barrier a call replaces, kept so the caller's can be put
/// back. Save-and-restore rather than a stack of environments, because the
/// nesting is the interpreter's own call stack already.
pub(crate) struct Saved {
    frames: Vec<Frame>,
    barrier: usize,
}

#[derive(Debug)]
pub(crate) struct Env {
    frames: Vec<Frame>,
    /// Frames below this index hold only what the enclosing `fn` may see.
    /// Zero at the top level and inside a lambda that was written there.
    barrier: usize,
}

impl Env {
    /// One frame to start with: the file's own, since a file is a block (§3).
    pub(crate) fn new() -> Env {
        Env {
            frames: vec![Frame::default()],
            barrier: 0,
        }
    }

    // ---- Scopes -----------------------------------------------------------

    pub(crate) fn push(&mut self) {
        self.frames.push(Frame::default());
    }

    pub(crate) fn pop(&mut self) {
        self.frames.pop();
        debug_assert!(!self.frames.is_empty(), "the file's frame was popped");
    }

    // ---- Calls ------------------------------------------------------------

    /// The frames a function declared here would capture, and the barrier it
    /// would inherit. What a lambda keeps; a `fn` keeps the frames and raises
    /// the barrier over all of them.
    pub(crate) fn capture(&self) -> (Vec<Frame>, usize) {
        (self.frames.clone(), self.barrier)
    }

    /// Swaps the callee's environment in and opens a frame for its parameters.
    pub(crate) fn enter(&mut self, captured: &[Frame], barrier: usize) -> Saved {
        let saved = Saved {
            frames: std::mem::replace(&mut self.frames, captured.to_vec()),
            barrier: std::mem::replace(&mut self.barrier, barrier),
        };
        self.push();
        saved
    }

    pub(crate) fn leave(&mut self, saved: Saved) {
        self.frames = saved.frames;
        self.barrier = saved.barrier;
    }

    // ---- Names ------------------------------------------------------------

    pub(crate) fn declare(&mut self, name: &str, value: Value, mutable: bool) {
        let frame = self.frames.last().expect("a frame is always open");
        frame
            .borrow_mut()
            .insert(name.to_string(), Binding { value, mutable });
    }

    pub(crate) fn get(&self, name: &str) -> Result<Value, LookupError> {
        self.lookup(name).map(|(value, _)| value)
    }

    /// A name's value and whether it may be assigned to — the second of which
    /// is what a `mut` argument is checked against at a call site (§5).
    pub(crate) fn lookup(&self, name: &str) -> Result<(Value, bool), LookupError> {
        for (index, frame) in self.frames.iter().enumerate().rev() {
            let Some(binding) = frame.borrow().get(name).cloned() else {
                continue;
            };
            // Under the barrier, a function is still reachable and anything
            // else would have to have been captured.
            if index < self.barrier && !binding.value.is_function() {
                return Err(LookupError::NotCaptured);
            }
            return Ok((binding.value, binding.mutable));
        }
        Err(LookupError::Unknown)
    }

    /// Assigns to the nearest binding of `name`. An inner frame's binding
    /// shadows an outer one for assignment exactly as it does for reading.
    pub(crate) fn assign(&mut self, name: &str, value: Value) -> Result<(), AssignError> {
        for (index, frame) in self.frames.iter().enumerate().rev() {
            let mut frame = frame.borrow_mut();
            let Some(binding) = frame.get_mut(name) else {
                continue;
            };
            if index < self.barrier && !binding.value.is_function() {
                return Err(AssignError::NotCaptured);
            }
            if !binding.mutable {
                return Err(AssignError::Immutable);
            }
            binding.value = value;
            return Ok(());
        }
        Err(AssignError::Unknown)
    }
}
