//! The instruction set, and what holds it.
//!
//! Three invariants explain nearly every choice here. None is enforced by the
//! Rust types alone, so they are worth reading first.
//!
//! **1. Types live in slots.** Every slot has a [`TypeId`] and every constant
//! knows its own, so no instruction repeats a type that could be looked up.
//! [`Rvalue::Cast`] does not say what it casts *to* — it converts to its
//! destination slot's type. There is exactly one place a type can be wrong.
//!
//! **2. Every operand is atomic** — a slot or a constant, never a nested
//! computation. That is A-normal form, and it is why §semantics' "operands
//! evaluate left to right" stops being a rule two back ends must each remember
//! and becomes the order of a statement list. `&&` and `||` are absent from
//! [`BinOp`] for the same reason: they short-circuit, so they lower to
//! [`Stmt::If`] and neither back end implements laziness.
//!
//! **3. Blocks nest; nothing jumps.** [`Stmt::Break`] and [`Stmt::Continue`]
//! name the innermost enclosing [`Stmt::While`]. There are no labels, no block
//! arguments, and no phi nodes — §control declined `goto` and wasm has none,
//! so there is no unstructured control flow to reconstruct.
//!
//! A [`Func`] owns its blocks, so a [`BlockId`] is function-local. Blocks live
//! in a flat `Vec`; the tree is in the [`Stmt`]s that name them.

use crate::consts::{ConstId, ConstPool};
use crate::sym::{Interner, Sym};
use crate::types::{FieldIdx, TypeDef, TypeId, Types};

/// Provenance: the CST node a piece of IR came from.
///
/// §comptime will widen this to a chain, so a diagnostic about an
/// instantiation can name both the generic body and the call that instantiated
/// it. Until `comptime` exists there is only ever one link.
pub type CstId = parser::NodeId;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FuncId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Slot(pub u32);

impl FuncId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl BlockId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl Slot {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A whole program: every monomorphic function, one type table, one constant
/// pool.
pub struct Program {
    pub funcs: Vec<Func>,
    pub types: Types,
    pub consts: ConstPool,
    pub syms: Interner,
    /// The synthetic function holding the file's top-level statements.
    /// §statements makes a file a block, so its trailing expression is this
    /// function's return value — which is the whole of goal §one-language's "a
    /// bare expression is a valid program", rather than a special case
    /// anywhere.
    pub entry: Option<FuncId>,
}

impl Program {
    pub fn func(&self, id: FuncId) -> &Func {
        &self.funcs[id.index()]
    }

    pub fn text(&self, sym: Sym) -> &str {
        self.syms.text(sym)
    }

    pub fn type_name(&self, ty: TypeId) -> String {
        type_name(&self.types, &self.syms, ty)
    }
}

/// Slot layout is `[params][captures][locals]`, so a lambda reads a captured
/// binding as an ordinary slot and the calling convention fills it from the
/// closure environment.
pub struct Func {
    pub name: Option<Sym>,
    /// `slots[0 .. params]`.
    pub params: u32,
    /// `slots[params .. params + captures]`.
    pub captures: u32,
    pub ret: TypeId,
    pub slots: Vec<SlotDef>,
    /// `blocks[0]` is the body.
    pub blocks: Vec<Block>,
    pub origin: CstId,
}

impl Func {
    pub fn body(&self) -> BlockId {
        BlockId(0)
    }

    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id.index()]
    }

    pub fn slot(&self, slot: Slot) -> &SlotDef {
        &self.slots[slot.index()]
    }

    pub fn slot_ty(&self, slot: Slot) -> TypeId {
        self.slots[slot.index()].ty
    }
}

pub struct SlotDef {
    pub ty: TypeId,
    /// `None` for a compiler-introduced temporary.
    pub name: Option<Sym>,
    pub kind: SlotKind,
    /// §statements: whether the value in this slot may be mutated *in place*
    /// through this binding — `let mut`, or §functions' `mut` parameter. Not
    /// about assignment: §statements leaves rebinding unrestricted on every
    /// binding, so `x = v` needs no permission and [`Stmt::Assign`] carries
    /// none.
    ///
    /// Nothing at run time consults this. It is the record of a rule checked
    /// during lowering, kept because it is a property of the slot and because
    /// a dump that did not show it would be missing the only difference
    /// between two otherwise identical parameters.
    pub mutable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotKind {
    Param,
    Capture,
    /// A `let` binding.
    Local,
    /// Introduced by A-normal form, or as the destination of a block or `if`
    /// used as a value.
    Temp,
}

impl SlotKind {
    pub fn name(self) -> &'static str {
        match self {
            SlotKind::Param => "param",
            SlotKind::Capture => "capture",
            SlotKind::Local => "local",
            SlotKind::Temp => "temp",
        }
    }
}

#[derive(Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Parallel to `stmts`. Beside them rather than inside so that a `Stmt`
    /// stays small and provenance can be dropped wholesale.
    pub spans: Vec<CstId>,
}

impl Block {
    pub fn new() -> Block {
        Block::default()
    }

    pub fn push(&mut self, stmt: Stmt, at: CstId) {
        self.stmts.push(stmt);
        self.spans.push(at);
    }
}

pub enum Stmt {
    Assign {
        dst: Slot,
        rvalue: Rvalue,
    },
    /// The assignment forms [`Stmt::Assign`] cannot express. Assigning to a
    /// plain local is always the former.
    Store {
        place: Place,
        value: Operand,
    },
    If {
        cond: Operand,
        then_: BlockId,
        else_: BlockId,
    },
    /// Run `header`, test `cond`, run `body`, repeat.
    ///
    /// The header exists because a loop condition is re-evaluated every
    /// iteration and may not be atomic, and ANF has nowhere else to put the
    /// computation. It lowers to wasm's `loop` + `br_if` directly.
    While {
        header: BlockId,
        cond: Operand,
        body: BlockId,
    },
    Break,
    Continue,
    Return(Option<Operand>),
    /// §statements' expression statement: evaluate, discard. Nothing marks
    /// the discard as deliberate, because §statements doesn't.
    Drop(Rvalue),
}

/// A slot or a constant. Invariant 2: nothing else is ever an operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operand {
    Slot(Slot),
    Const(ConstId),
}

pub enum Rvalue {
    Use(Operand),
    Unary(UnOp, Operand),
    Binary(BinOp, Operand, Operand),
    /// `x as T`, explicit and trapping (§expressions). The target is the
    /// destination slot's type — invariant 1.
    Cast(Operand),
    Call {
        func: FuncId,
        args: Vec<Operand>,
    },
    /// A call through a function *value* — a lambda, or a `fn` bound to a
    /// name. On wasm this is `call_indirect` (§functions, §wasm); when
    /// §objects brings dynamic dispatch it will be the same node.
    CallIndirect {
        callee: Operand,
        args: Vec<Operand>,
    },
    /// Fields in declaration order. §expressions' left-to-right evaluation of
    /// the *written* order already happened in the statements above this one.
    MakeStruct(Vec<Operand>),
    Field {
        base: Operand,
        field: FieldIdx,
    },
    MakeCell(Operand),
    CellGet(Operand),
    /// Every function value is one of these. A plain `fn` referred to by name
    /// has an empty capture list.
    MakeClosure {
        func: FuncId,
        captures: Vec<Operand>,
    },
}

pub enum Place {
    Field { base: Slot, field: FieldIdx },
    Cell(Slot),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    /// Signed and float only (§expressions) — negating an unsigned value is a
    /// type error, not a trap.
    Neg,
    Not,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl UnOp {
    pub fn name(self) -> &'static str {
        match self {
            UnOp::Neg => "neg",
            UnOp::Not => "not",
        }
    }
}

impl BinOp {
    pub fn name(self) -> &'static str {
        match self {
            BinOp::Add => "add",
            BinOp::Sub => "sub",
            BinOp::Mul => "mul",
            BinOp::Div => "div",
            BinOp::Rem => "rem",
            BinOp::Eq => "eq",
            BinOp::Ne => "ne",
            BinOp::Lt => "lt",
            BinOp::Le => "le",
            BinOp::Gt => "gt",
            BinOp::Ge => "ge",
        }
    }

    /// The spelling in a diagnostic, which is the source's rather than the
    /// IR's.
    pub fn spelling(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
        }
    }

    /// Whether the result is `bool` rather than the operand type.
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        )
    }
}

/// How a type is written in a dump or a diagnostic.
pub fn type_name(types: &Types, syms: &Interner, ty: TypeId) -> String {
    match types.get(ty) {
        TypeDef::Unit => "()".to_string(),
        TypeDef::Bool => "bool".to_string(),
        TypeDef::Char => "char".to_string(),
        TypeDef::Str => "Str".to_string(),
        TypeDef::Int { signed, bits } => format!("{}{}", if *signed { "s" } else { "u" }, bits),
        TypeDef::Float { bits } => format!("f{bits}"),
        TypeDef::Fn { params, ret } => {
            let params: Vec<_> = params
                .iter()
                .map(|param| type_name(types, syms, *param))
                .collect();
            let params = params.join(" ");
            if matches!(types.get(*ret), TypeDef::Unit) {
                format!("(fn ({params}))")
            } else {
                format!("(fn ({params}) -> {})", type_name(types, syms, *ret))
            }
        }
        TypeDef::Struct(def) => match def.name {
            Some(name) => syms.text(name).to_string(),
            None => "struct".to_string(),
        },
        TypeDef::Cell(inner) => format!("(cell {})", type_name(types, syms, *inner)),
        TypeDef::Error => "?".to_string(),
    }
}
