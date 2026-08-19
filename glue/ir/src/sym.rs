//! Interned names.
//!
//! Names exist in core IR for diagnostics and dumps and for nothing else —
//! resolution has already happened, so nothing is ever *looked up* by name.
//! They are interned anyway because [`crate::types::TypeDef`] has to be
//! hashable for structural interning, and a `String` inside one would make
//! every hash a walk over the text.

use std::collections::HashMap;

/// An interned name. Cheap to copy and compare, meaningless without the
/// [`Interner`] that produced it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Sym(pub u32);

#[derive(Default)]
pub struct Interner {
    texts: Vec<String>,
    lookup: HashMap<String, Sym>,
}

impl Interner {
    pub fn new() -> Interner {
        Interner::default()
    }

    pub fn intern(&mut self, text: &str) -> Sym {
        if let Some(&sym) = self.lookup.get(text) {
            return sym;
        }
        let sym = Sym(self.texts.len() as u32);
        self.texts.push(text.to_string());
        self.lookup.insert(text.to_string(), sym);
        sym
    }

    pub fn text(&self, sym: Sym) -> &str {
        &self.texts[sym.0 as usize]
    }
}
