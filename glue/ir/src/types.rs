//! The type table.
//!
//! Two kinds of entry, because §types is nominal:
//!
//! * **Structural** — primitives, unit, `fn(T, …) -> R`, and the internal
//!   [`TypeDef::Cell`]. These have no identity beyond their shape, so they are
//!   interned and two spellings of `fn(u64) -> u64` are one [`TypeId`].
//! * **Nominal** — a struct. §types says every evaluation of a `struct { … }`
//!   expression produces a *fresh* type, so [`Types::fresh_struct`] never
//!   interns. Two structs with identical fields are different types, which is
//!   the nominal rule reached from the representation side: identity comes
//!   from the act of construction rather than from the shape constructed.
//!
//! When §comptime lands, `Pair(u64, Str)` will be one type not because two
//! identical structs get merged but because the instantiation cache runs the
//! body once. Nothing here changes to accommodate that.

use std::collections::HashMap;

use parser::NodeId;

use crate::sym::Sym;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TypeId(pub u32);

impl TypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The position of a field within its struct.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FieldIdx(pub u32);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeDef {
    /// One value, one inhabitant (§types). Not an absence.
    Unit,
    Bool,
    Char,
    /// UTF-8 bytes (§lexical).
    Str,
    /// `u8`…`u64`, `s8`…`s64`. `s` rather than `i`, matching wasm's own
    /// instruction suffixes (§lexical).
    Int {
        signed: bool,
        bits: u8,
    },
    Float {
        bits: u8,
    },
    /// `fn(T, …) -> R` (§functions). One representation covers both a `fn` and
    /// a lambda: every function value is a code reference plus an environment,
    /// and a plain `fn` has an empty one.
    Fn {
        params: Vec<TypeId>,
        ret: TypeId,
    },
    Struct(StructDef),
    /// A one-word heap box holding a `T`. **IR-internal** — no Glue program
    /// can name one. Lowering introduces cells for bindings that are both
    /// captured by a lambda and assigned, so that §functions' promise that
    /// captured bindings outlive their frame has somewhere to be true.
    Cell(TypeId),
    /// §comptime: the type whose values are types. The only type with **no
    /// runtime representation** — a value of it may be computed, bound, and
    /// passed during elaboration, and may not cross into a running program.
    /// Nothing in core IR is ever typed with it, which is the invariant
    /// `elab` enforces by refusing to pin one.
    Type,
    /// Poison. Produced where a diagnostic has already been reported, and
    /// compatible with everything, so one mistake yields one message.
    Error,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct StructDef {
    /// For diagnostics and dumps, never for identity. `None` until something
    /// gives the type a name — the anonymous `struct { … }` expression
    /// (§comptime) has none until a `let` binds it.
    pub name: Option<Sym>,
    pub fields: Vec<FieldDef>,
    /// §types: identity is the site that constructed it.
    pub origin: NodeId,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FieldDef {
    pub name: Sym,
    pub ty: TypeId,
}

pub struct Types {
    defs: Vec<TypeDef>,
    /// Structural entries only. A struct never appears here.
    interned: HashMap<TypeDef, TypeId>,
}

impl Types {
    pub fn new() -> Types {
        Types {
            defs: Vec::new(),
            interned: HashMap::new(),
        }
    }

    /// Interns a structural type. Passing a [`TypeDef::Struct`] would merge
    /// two types §types says are distinct, so it is refused.
    pub fn intern(&mut self, def: TypeDef) -> TypeId {
        debug_assert!(
            !matches!(def, TypeDef::Struct(_)),
            "a struct is nominal — use `fresh_struct`"
        );
        if let Some(&id) = self.interned.get(&def) {
            return id;
        }
        let id = TypeId(self.defs.len() as u32);
        self.defs.push(def.clone());
        self.interned.insert(def, id);
        id
    }

    /// Allocates a struct type with no fields yet.
    ///
    /// Fields are filled in afterwards by [`Types::set_fields`], because a
    /// struct may mention itself or one declared later in the same block — and
    /// under §types' reference semantics that is an ordinary thing to write
    /// rather than an infinite size.
    pub fn fresh_struct(&mut self, name: Option<Sym>, origin: NodeId) -> TypeId {
        let id = TypeId(self.defs.len() as u32);
        self.defs.push(TypeDef::Struct(StructDef {
            name,
            fields: Vec::new(),
            origin,
        }));
        id
    }

    /// Names a struct that has none.
    ///
    /// §comptime makes `struct Point { … }` sugar for
    /// `let Point = struct { … };`, and a name is for diagnostics and dumps
    /// rather than for identity ([`StructDef::name`]) — so the sugar holds
    /// exactly when the binding lends its name to the type it binds. An
    /// already-named struct keeps its name: `let A = Point;` is a second name
    /// for one type (§types), not a rename.
    pub fn name_struct(&mut self, id: TypeId, name: Sym) {
        if let TypeDef::Struct(def) = &mut self.defs[id.index()]
            && def.name.is_none()
        {
            def.name = Some(name);
        }
    }

    pub fn set_fields(&mut self, id: TypeId, fields: Vec<FieldDef>) {
        match &mut self.defs[id.index()] {
            TypeDef::Struct(def) => def.fields = fields,
            _ => unreachable!("set_fields on something that isn't a struct"),
        }
    }

    pub fn get(&self, id: TypeId) -> &TypeDef {
        &self.defs[id.index()]
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn struct_def(&self, id: TypeId) -> Option<&StructDef> {
        match self.get(id) {
            TypeDef::Struct(def) => Some(def),
            _ => None,
        }
    }

    pub fn fn_sig(&self, id: TypeId) -> Option<(Vec<TypeId>, TypeId)> {
        match self.get(id) {
            TypeDef::Fn { params, ret } => Some((params.clone(), *ret)),
            _ => None,
        }
    }

    pub fn is_error(&self, id: TypeId) -> bool {
        matches!(self.get(id), TypeDef::Error)
    }

    /// §comptime's boundary: everything but [`TypeDef::Type`] may become a
    /// runtime value.
    pub fn has_runtime_representation(&self, id: TypeId) -> bool {
        !matches!(self.get(id), TypeDef::Type)
    }

    pub fn is_numeric(&self, id: TypeId) -> bool {
        matches!(self.get(id), TypeDef::Int { .. } | TypeDef::Float { .. })
    }

    pub fn is_integer(&self, id: TypeId) -> bool {
        matches!(self.get(id), TypeDef::Int { .. })
    }

    pub fn is_float(&self, id: TypeId) -> bool {
        matches!(self.get(id), TypeDef::Float { .. })
    }

    /// §expressions: unary `-` is defined on signed and float types only.
    pub fn is_signed_or_float(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            TypeDef::Int { signed: true, .. } | TypeDef::Float { .. }
        )
    }

    /// Whether values of this type can be ordered with `<`, `<=`, `>`, `>=`.
    pub fn is_ordered(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            TypeDef::Int { .. } | TypeDef::Float { .. } | TypeDef::Char | TypeDef::Str
        )
    }

    /// Type equality, with `Error` compatible with everything so that a
    /// reported mistake produces one diagnostic rather than a cascade.
    pub fn compatible(&self, a: TypeId, b: TypeId) -> bool {
        a == b || self.is_error(a) || self.is_error(b)
    }
}

impl Default for Types {
    fn default() -> Types {
        Types::new()
    }
}
