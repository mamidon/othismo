//! Bindings, and the scopes they live in.
//!
//! A stack of maps, innermost last. §12 will have more to say about names, but
//! the two rules that already exist are both here:
//!
//! * **Shadowing is allowed, including in the same scope** (§3). `let input =
//!   parse(input);` is the natural way to write a narrowing pipeline without
//!   inventing `input2`. That falls out of `declare` overwriting rather than
//!   refusing — the old binding is unreachable but the value it held is still
//!   alive as long as the initializer that read it needed it.
//! * **`mut` gates mutation, not rebinding** (§3). Assigning to a binding
//!   declared without `mut` is an error; declaring it again with `let` is not.
//!
//! A block pushes a scope and pops it, so a binding made in a `while` body is
//! fresh each iteration — which is the whole reason §5 can say the classic
//! loop-variable capture trap is absent.

use std::collections::HashMap;

use crate::value::Value;

/// A name, what it holds, and whether it may be assigned to.
#[derive(Clone, Debug)]
struct Binding {
    value: Value,
    mutable: bool,
}

/// Why an assignment couldn't happen. The caller turns this into a
/// [`crate::RuntimeError`], because it is the one that knows the span.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AssignError {
    Unknown,
    Immutable,
}

#[derive(Debug)]
pub(crate) struct Env {
    scopes: Vec<HashMap<String, Binding>>,
}

impl Env {
    /// One scope to start with: the file's own, since a file is a block (§3).
    pub(crate) fn new() -> Env {
        Env {
            scopes: vec![HashMap::new()],
        }
    }

    pub(crate) fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn pop(&mut self) {
        self.scopes.pop();
        debug_assert!(!self.scopes.is_empty(), "the file's scope was popped");
    }

    pub(crate) fn declare(&mut self, name: &str, value: Value, mutable: bool) {
        let scope = self.scopes.last_mut().expect("a scope is always open");
        scope.insert(name.to_string(), Binding { value, mutable });
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .map(|binding| &binding.value)
    }

    /// Assigns to the nearest binding of `name`. An inner scope's binding
    /// shadows an outer one for assignment exactly as it does for reading.
    pub(crate) fn assign(&mut self, name: &str, value: Value) -> Result<(), AssignError> {
        let binding = self
            .scopes
            .iter_mut()
            .rev()
            .find_map(|scope| scope.get_mut(name))
            .ok_or(AssignError::Unknown)?;
        if !binding.mutable {
            return Err(AssignError::Immutable);
        }
        binding.value = value;
        Ok(())
    }
}
