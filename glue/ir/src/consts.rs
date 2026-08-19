//! The constant pool: where comptime results land.
//!
//! Today this holds literals, because §14's `comptime` has no token yet. The
//! shape is chosen for what it has to hold once it does.
//!
//! A [`Const::Struct`] holds `ConstId`s rather than nested values, so **sharing
//! is the same id appearing twice, and a cycle is an id that transitively
//! reaches itself**. That matters because §6 gives structs reference semantics,
//! which makes sharing observable, and because core IR imposes no restriction
//! on what a comptime value may be. An index arena represents graphs Rust
//! ownership cannot, with no `Rc<RefCell<…>>` and no unsafe — the pool is the
//! *frozen* result of evaluation, not the evaluator's working heap.
//!
//! Floats are stored as bits so that a `Const` can be `Eq` and `Hash`, which is
//! what lets two identical constants share an id. `0.0` and `-0.0` therefore
//! get separate entries, which is correct: they are distinguishable values.

use std::collections::HashMap;
use std::rc::Rc;

use crate::types::TypeId;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ConstId(pub u32);

impl ConstId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Const {
    Unit,
    Bool(bool),
    /// Already pinned to a concrete width (§1). The bits are the value as
    /// stored: sign-extended for a signed type, zero-extended otherwise.
    Int {
        ty: TypeId,
        bits: u64,
    },
    Float {
        ty: TypeId,
        bits: u64,
    },
    Char(char),
    Str(Rc<str>),
    Struct {
        ty: TypeId,
        fields: Vec<ConstId>,
    },
}

#[derive(Default)]
pub struct ConstPool {
    values: Vec<Const>,
    lookup: HashMap<Const, ConstId>,
}

impl ConstPool {
    pub fn new() -> ConstPool {
        ConstPool::default()
    }

    pub fn add(&mut self, value: Const) -> ConstId {
        if let Some(&id) = self.lookup.get(&value) {
            return id;
        }
        let id = ConstId(self.values.len() as u32);
        self.values.push(value.clone());
        self.lookup.insert(value, id);
        id
    }

    pub fn get(&self, id: ConstId) -> &Const {
        &self.values[id.index()]
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
