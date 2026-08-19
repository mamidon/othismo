# Core IR — the structures

> **Status:** Draft for review, 2026-08-18. Not yet folded into [`core-ir.md`](core-ir.md).
>
> A concrete rendering of the representation `core-ir.md` specifies in prose. Two items at
> the end **sharpen decisions already recorded there** and should be reconciled before this
> is merged or discarded.

Here's the whole thing. It's about 150 lines of Rust, which is itself worth noticing — the
smallness is what the five decisions bought.

## The invariants that determine the shape

Three rules explain almost every choice below:

1. **Types live in slots.** Every slot has a `TypeId`, every constant knows its own type,
   and therefore no instruction ever carries a type that could be looked up. `Cast` doesn't
   say what it casts *to* — it converts to its destination slot's type. This is only sound
   because decision 5 made slots typed, and it means there's exactly one place a type can
   be wrong.
2. **Every operand is atomic** — a slot or a constant, never a nested computation. That's
   decision 1.
3. **Blocks nest; nothing jumps.** `Break`/`Continue` name the innermost enclosing `While`.
   There are no labels, no block arguments, no phis.

## The structures

```rust
// ---- ids -------------------------------------------------------------------
// All newtype u32. Indices, not pointers — decision 4's real payoff.

pub struct FuncId(u32);    pub struct TypeId(u32);
pub struct ConstId(u32);   pub struct BlockId(u32);   // block ids are function-local
pub struct Slot(u32);      pub struct FieldIdx(u32);
pub struct Sym(u32);       // interned name, diagnostics only
pub struct CstId(u32);     // provenance into the parser's tree
pub struct InstId(u32);    // an instantiation, for the call-site chain
pub struct HostId(u32);    // a §13 host import

// ---- program ---------------------------------------------------------------

pub struct Program {
    pub funcs:  Vec<Func>,
    pub types:  Types,
    pub consts: ConstPool,
    pub insts:  Vec<Instantiation>,
    pub hosts:  Vec<HostImport>,
    pub entry:  Option<FuncId>,
    /// §14's memo. This is the whole of monomorphization.
    instantiations: HashMap<(DeclId, Vec<ConstId>), FuncId>,
}

// ---- types -----------------------------------------------------------------

pub enum TypeDef {
    Unit,
    Bool,
    Char,
    Int   { signed: bool, bits: u8 },      // u8..u64 / s8..s64
    Float { bits: u8 },                    // f32, f64
    Str,
    Fn    { params: Vec<TypeId>, ret: TypeId },
    Struct(StructDef),                     // nominal
    Cell(TypeId),                          // IR-internal; see captures below
}

pub struct StructDef {
    pub name:   Option<Sym>,               // "Pair(u64, Str)" for diagnostics
    pub fields: Vec<(Sym, TypeId)>,
    pub origin: Origin,                    // §6: identity IS the allocation site
}

pub struct Types { defs: Vec<TypeDef>, interned: HashMap<TypeDef, TypeId> }

impl Types {
    /// Primitives, `Fn`, `Cell` — structural, so interning is correct.
    pub fn intern(&mut self, d: TypeDef) -> TypeId { … }
    /// The ONLY way to make a struct. Never interned: §6 says every evaluation
    /// of `struct { … }` yields a fresh type.
    pub fn fresh_struct(&mut self, s: StructDef) -> TypeId { … }
}

// ---- functions -------------------------------------------------------------

pub struct Func {
    pub name:     Option<Sym>,
    pub params:   u32,          // slots[0 .. params]
    pub captures: u32,          // slots[params .. params+captures]
    pub ret:      TypeId,       // remaining slots are locals and ANF temporaries
    pub slots:    Vec<SlotDef>,
    pub blocks:   Vec<Block>,   // blocks[0] is the body
    pub origin:   Origin,
}

pub struct SlotDef { pub ty: TypeId, pub name: Option<Sym> }

pub struct Block {
    pub stmts: Vec<Stmt>,
    pub spans: Vec<CstId>,      // parallel to stmts; droppable in a release build
}

// ---- instructions ----------------------------------------------------------

pub enum Stmt {
    Assign { dst: Slot, rvalue: Rvalue },
    Store  { place: Place, value: Operand },
    If     { cond: Operand, then_: BlockId, else_: BlockId },
    /// Run `header`, test `cond`, run `body`, repeat. The header is what makes
    /// a non-atomic loop condition expressible in ANF.
    While  { header: BlockId, cond: Operand, body: BlockId },
    Break,
    Continue,
    Return(Option<Operand>),
    Drop(Rvalue),               // §3's expression statement
    Trap(TrapKind),             // provisional; §9 owns the taxonomy
}

pub enum Operand { Slot(Slot), Const(ConstId) }

pub enum Rvalue {
    Use(Operand),
    Unary  (UnOp, Operand),
    Binary (BinOp, Operand, Operand),
    Cast   (Operand),                                    // target = dst slot's type
    Call         { func: FuncId,   args: Vec<Operand> },
    CallIndirect { callee: Operand, args: Vec<Operand> },
    CallHost     { import: HostId, args: Vec<Operand> }, // §13
    MakeStruct   (Vec<Operand>),                         // type = dst slot's type
    Field        { base: Operand, field: FieldIdx },
    Index        { base: Operand, index: Operand },      // Str for now; §6
    MakeCell     (Operand),
    CellGet      (Operand),
    MakeClosure  { func: FuncId, captures: Vec<Operand> },
}

/// Only the forms that `Assign` can't express — ANF guarantees the base is atomic,
/// so `a.b.c = v` is already `t = a.b; t.c = v`.
pub enum Place {
    Field { base: Slot, field: FieldIdx },
    Index { base: Slot, index: Operand },
    Cell  (Slot),
}

pub enum UnOp  { Neg, Not }
pub enum BinOp { Add, Sub, Mul, Div, Rem, Eq, Ne, Lt, Le, Gt, Ge }
// No And/Or — §2 short-circuits, so `&&` and `||` lower to `If`.

// ---- constants: where comptime results land --------------------------------

pub struct ConstPool { values: Vec<Const> }

pub enum Const {
    Unit,
    Bool(bool),
    Int    { ty: TypeId, bits: u64 },      // zero/sign-extended
    Float  { ty: TypeId, bits: u64 },
    Char(char),
    Str(Rc<str>),
    Struct { ty: TypeId, fields: Vec<ConstId> },
    Fn     { func: FuncId, captures: Vec<ConstId> },
}

// ---- provenance ------------------------------------------------------------

pub struct Origin { pub cst: CstId, pub inst: Option<InstId> }

pub struct Instantiation {
    pub decl: DeclId, pub args: Vec<ConstId>,
    pub call_site: CstId, pub parent: Option<InstId>,   // the chain §14 wants
}
```

## Reading the dumps

The debug rendering is s-expressions. Every IR node is a form:

```
(kind <desc>)                    ; leaf
(kind <desc>
  <child>                        ; one child per line, children may nest
  <child>)
```

with four conventions to keep it from drowning in parens:

- **A slot in operand position is its bare name.** `i`, `t0`. Anything parenthesized is a
  node, so there is no ambiguity.
- **A constant is `(const …)`**, carrying its suffixed literal so its type is visible.
- **`Rvalue::Use` is elided.** A bare operand where an rvalue is expected is a use.
- **The integer after `slot`, `block`, `header`, and `body` is the arena index**, so a dump
  can be cross-referenced against the structures above. `;` begins a comment.

## Worked example: loops, slots, ANF temporaries

```
fn count_to(n: u64) -> u64 {
  let mut i = 0;
  while i < n * 2 {
    i = i + 1;
  }
  i
}
```

```scheme
(func count_to (u64) -> u64
  (slot 0 n  u64  param)
  (slot 1 i  u64)
  (slot 2 t0 u64  temp)
  (slot 3 t1 bool temp)
  (block 0
    (assign i (const 0u64))
    (while
      (header 1                       ; re-runs every iteration
        (assign t0 (mul n (const 2u64)))
        (assign t1 (lt i t0)))
      (cond t1)
      (body 2
        (assign i (add i (const 1u64)))))
    (return i)))
```

The `n * 2` lands in the header, not before the loop, because the header is what
re-evaluates. That's the price of ANF and it's visible right here — a real compiler would
hoist it, and Glue won't, because there's no optimizer (decision 5's premise).

Note that blocks nest in the dump rather than being listed flat and referenced by id. That
is a property of the *rendering*, not of the data: `Func` holds a flat `Vec<Block>` and
`Stmt::If` / `Stmt::While` hold `BlockId`s. The dump can nest them because invariant 3 says
control flow is a tree — which is the same fact that makes the wasm back end a translation
rather than a reconstruction.

## Worked example: captures and cells

```
fn counter() -> fn() -> u64 {
  let mut n = 0;
  () -> { n = n + 1; n }
}
```

```scheme
(func counter () -> (fn () -> u64)
  (slot 0 t0 u64            temp)
  (slot 1 n  (cell u64))              ; `n` is mut AND captured -> cell
  (slot 2 f  (fn () -> u64) temp)
  (block 0
    (assign t0 (const 0u64))
    (assign n  (makecell t0))
    (assign f  (closure counter.λ0
                 (captures n)))
    (return f)))

(func counter.λ0 () -> u64
  (slot 0 n  (cell u64) capture)
  (slot 1 t0 u64        temp)
  (slot 2 t1 u64        temp)
  (block 0
    (assign t0 (cellget n))
    (assign t1 (add t0 (const 1u64)))
    (store (cell n) t1)               ; §3's assignment statement
    (assign t0 (cellget n))           ; the block's trailing expression
    (return t0)))
```

Slot layout is `[params][captures][locals]`, so a lambda reads its captures as ordinary
slots and the calling convention populates them from the closure environment.

A lambda has no source name, so the dump gives it a synthetic one derived from its
enclosing function — `counter.λ0`. In the data it is an ordinary `FuncId` like any other;
the name exists for diagnostics and for dumps, and `Func::name` is `Option<Sym>` for
exactly this reason.

## Two things this sharpened, versus what I wrote in the doc

**A capture needs a cell only if the binding is `mut`.** The doc says captured bindings get
cells; that's stronger than necessary. A non-`mut` binding can never be assigned, so copying
its value into the closure environment is observationally identical — and under §6's
reference semantics, copying a struct binding copies the *reference*, so mutation of the
object stays visible either way. What the cell protects is assignment to the *name*, which
requires `mut`.

That also makes §5's per-iteration promise fall out more cheaply than I described: a
non-`mut` `let` in a loop body is copied fresh into each closure, per iteration, with no
allocation at all. Only captured `mut` bindings pay.

**The constant pool needs no `Rc<RefCell>` and no unsafe.** `Const::Struct` holds
`Vec<ConstId>`, so sharing is "the same id twice" and a cycle is just an id that
transitively reaches itself. Index arenas represent cyclic object graphs that Rust
ownership can't. That's decision 3 — no restrictions on comptime values — getting cheaper
than I expected: the pool is the *frozen* result of evaluation, and freezing is where the
evaluator's working heap gets interned into ids.

---

Want me to fold both into `core-ir.md` and stand up an `ir/` crate with this as the
skeleton?
