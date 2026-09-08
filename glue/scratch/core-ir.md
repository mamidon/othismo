# Core IR

> **Status:** Decided 2026-08-18. Items marked **Open** are known gaps, not oversights.
>
> Not a section of the construct checklist — core IR is not a language construct, it is
> the shared artifact the two back ends consume. Named and required by
> [§comptime](constructs/14-metaprogramming-and-tooling.md#what-reaches-the-back-ends);
> specified here.
>
> **Implemented in `../ir/`, `../elab/`, and `../eval/`.** The crates are this document
> executable: `elab::lower` takes a CST and returns a `Program` plus diagnostics,
> `ir::dump` renders it, and `eval::eval` runs it. Where the two disagree the crates are
> wrong, except where this document says otherwise and dates it. See *Crate layout*, below,
> for why that is three crates and not one.

## What this is

```
source → CST → typed tree → [elaboration] → core IR → { interpreter, wasm }
```

Core IR is the concrete form of goal §both-modes' shared front end. It is **typed,
monomorphic, and free of comptime, generics, and `Type`**. Both back ends consume it, and
goal §both-modes' conformance suite attaches to it: one program, one core IR, two
executions, identical observable results.

**The CST is not rewritten.** Elaboration reads it and emits this, because the CST is the
language server's tree and must keep the property `parser` opens with — every byte of the
source reachable from the tree. Core IR nodes carry provenance back to CST nodes instead.

Core IR has **no syntax**. It is a Rust data structure, and any textual rendering of it is
a debugging aid with no normative status.

---

## Decisions

### 1. A-normal form

**Every operand of every operation is atomic** — a slot or a constant. Never a nested
computation. Intermediates get compiler-introduced slots.

```
let d = (a * a) - (4 * b * c);
```

becomes

```
t0 = a * a
t1 = 4 * b
t2 = t1 * c
d  = t0 - t2
```

Three reasons, in order of force:

- **§semantics' evaluation order stops being a rule and becomes the shape of the data.**
  In a nested tree, "operands evaluate left to right" is a fact the interpreter and the
  wasm back end must each independently remember, and §semantics is precisely the register
  of things two implementations disagree about silently. In ANF there is one statement
  list and one order to walk it.
- **Short-circuiting stops being a special case.** `a && b` lowers to an `if`, because `b`
  is a computation and computations cannot be operands. Neither back end implements
  laziness for `&&`; both implement `if`, once.
- **It keeps the suspension door open.** §control and §modules both leave async
  unresolved, and §control notes that a structured region is something a transform can
  split. In ANF every live value is a named slot, so the state at any statement boundary
  *is* the live slot set — which is what a state-machine transform or an interpreter-side
  continuation needs. Nested operand stacks would have to be reconstructed.

ANF flattens **expressions only**. Control flow stays structured and nested, which is
§control's decision and wasm's shape both.

**The one wobble:** a `while` condition is re-evaluated per iteration and may not be
atomic, so it is a *block* ending in a slot rather than a slot. That lowers to wasm's
`loop` + `br_if` directly, but it is the place the "flat statement list" story bends, and
it is written down here so it is not discovered.

### 2. One evaluator

**The interpreter is the comptime engine.** §comptime requires something to execute Glue
during compilation; rather than a second evaluator over the typed tree, elaboration lowers
a body to core IR and runs the existing interpreter on it.

The alternative — a comptime evaluator and a runtime evaluator — is where Zig has
historically leaked semantic differences between the two stages. One evaluator makes that
divergence unrepresentable, which is goal §both-modes' thesis applied one level up.

The chicken-and-egg (elaboration needs comptime results; comptime needs elaborated code)
resolves the way §comptime already resolves it: demand-driven, with the instantiation
cache doing the memoizing. A comptime call reaches a callee, the callee is elaborated at
the point its comptime parameters are bound, the result is cached on `(declaration,
comptime arguments)`.

**Two configurations, not two programs.** The evaluator takes a mode:

| | comptime | runtime |
| --- | --- | --- |
| Fuel budget | §comptime's, finite | none |
| Recursion depth cap | §comptime's | stack, then trap |
| Host imports (§modules) | denied — comptime is hermetic | permitted |
| A trap (§errors) | a diagnostic | a wasm trap |
| `Type` values | permitted | unrepresentable |

The last two rows are the payoff. **A trap at comptime is a compile error**, with one
implementation of each check and only the reporting differing — overflow, division by
zero, and depth exhaustion get their §errors answer in one place rather than two.

**The value domain is shared; comptime's is a superset.** Two variants exist only in the
comptime configuration: `Type`, and §lexical's unpinned integer constant. Nothing else
differs.

### 3. No restrictions on comptime values

A comptime value that becomes a runtime value may be any shape the evaluator can build —
including a cyclic, shared object graph, which §types' reference semantics makes
observable. No restriction is imposed now.

This is affordable because of how the wasm back end will materialize it: **emit
initialization code that rebuilds the graph at instance start**, rather than laying it out
in a static data segment. Allocate, then fix up references. Cycles and sharing both fall
out; the cost is startup time, and a static fast path for flat acyclic aggregates can be
added later without a language-visible rule.

Restrictions were only ever needed to make static layout work. Declining static layout as
the *only* strategy declines the restriction with it.

The interpreter path is trivial — the object graph is already in its heap.

**Open:** whether the fast path is worth building, and what identifies the cases it covers.

### 4. Not serialized

Core IR is a Rust crate with a public API, consumed by the interpreter, the wasm back end,
and the language server. It is not written to disk, so there is no versioned byte format,
no POD-only constraint, and no self-describing encoding to design.

**Indices are used anyway**, for a reason unrelated to disk. Decision 2 means the
elaborator mutates the program — adding instantiations — while the evaluator is running
inside it. Nodes holding Rust references would make that an immediate borrow conflict;
`u32` indices let the evaluator copy small values out and hold no borrow across an
instantiation.

The consequence worth preserving: **keep IR nodes free of native heap pointers even though
nothing requires it.** If goal §liveness ever ships an interpreter tier inside a running
image — which is serialization by another name — the expensive-to-change part is already
right.

**Revised 2026-08-18.** This originally said the constant pool would be the one place
holding live Rust values, and that turned out not to be necessary. A `Const::Struct` holds
`ConstId`s, so sharing is the same id appearing twice and a cycle is an id that
transitively reaches itself. An index arena represents graphs Rust ownership cannot, with
no `Rc<RefCell<…>>` and no unsafe — which makes decision 3 cheaper than it looked, since
the pool is the *frozen* result of evaluation rather than the evaluator's working heap.

### 5. Slots, not SSA

Every local is a numbered slot with a static type; parameters are the first slots. Slots
are function-scoped and reused across iterations.

SSA's payoff is optimization, and goal §both-modes declines to be maximally fast. Phi
nodes buy nothing without a mid-level optimizer. Slots additionally map **one-to-one onto
wasm locals**, which is a consequence of §types rather than luck: primitives are scalars
and aggregates are references, so every Glue type has a single-word runtime
representation.

Temporaries introduced by ANF happen to be single-assignment. User `mut` bindings are not.
That distinction is a description of what falls out, not a rule the IR enforces.

---

## Captured bindings do not live in slots

**The one place decision 5 collides with a decision already made.** §functions promises
that a `let` inside a loop body is a fresh binding per iteration, captured separately —
the classic loop-variable trap, absent by construction. Function-scoped slots reintroduce
it: one slot, reused, shared by every lambda the loop creates.

So capture analysis — which §scope already lists as a separate static pass — marks each
binding, and:

- an **uncaptured** binding is a slot, and costs nothing;
- a binding that is **captured and assigned** is a heap cell allocated at its point of
  declaration, with the slot holding a reference to the cell;
- a binding that is **captured and never assigned** is copied into the closure
  environment. Every copy stays equal forever, so the sharing a cell provides is
  unobservable — and under §types' reference semantics copying a struct binding copies
  the *reference*, so mutation of the object is visible either way. What a cell protects
  is assignment to the **name**.

A `let` in a loop body allocates a fresh cell per iteration, so §functions' per-iteration
semantics falls out rather than needing a rule, and §functions' "captured bindings
outlive the frame that created them" is satisfied by the same mechanism. In the copied
case it falls out even more cheaply: each iteration copies its own value and there is no
allocation at all.

**Assigned, not `mut` — and that is a place two sections disagree.** §statements says
rebinding is unrestricted on any binding (`x = Foo::create(); // fine — rebinding is
unrestricted`) and that `mut` gates only in-place mutation. §functions says "the binding
must be `mut` for the lambda to mutate it at all (§statements)". Under §statements' rule
a lambda can rebind a non-`mut` captured binding, so the criterion has to be assignment.
**Decided 2026-08-18: assignment.** If §functions turns out to mean that assigning a
captured binding requires `mut`, this tightens and fewer cells are allocated; nothing else
moves.

**A `mut` parameter is a permission, not a representation. Decided 2026-08-19.**
§functions leaves open whether a `mut` parameter is by reference or copy-in/copy-out, and
§statements answers the question it was really asking: `mut` gates *in-place mutation*, so
a `mut` parameter is the callee's permission to mutate the argument through that binding,
and §statements' consuming rule is that the call site must pass a `mut` binding. What the
caller observes afterwards is whatever §types makes observable through the value it passed
— for a struct, the mutation itself; for a scalar, nothing, because there is nothing to
alias.

So the IR needs no write-back node and no second calling convention. `SlotDef::mutable`
records the permission, elaboration checks it, and nothing at run time consults it. The
check travels with the *declaration* rather than the type, because §functions gives a `fn`
type no `mut` — so a call through a function value is unchecked, and stays so until
§functions grows the syntax to say otherwise.

Both halves are syntactic, which is deliberate — §lexical asks for exactly that, since "a
rule needing type inference or dataflow to answer is a rule the two back ends will
eventually disagree about." The scan over-approximates by ignoring shadowing, so the cost
of imprecision is a cell nobody needed rather than a missing one.

**Per binding, not per frame.** A per-frame environment would re-share everything declared
in one iteration, which is the bug in a different shape.

This adds `MakeCell` and `CellGet` to the IR, plus a `Place::Cell` for the write side —
assignment through a cell is an ordinary `Store`, not a third instruction. A closure's
captures are cell references for the cell case and slot copies otherwise. It is the upvalue
treatment Lua uses and the `let`-versus-`var` distinction JavaScript arrived at the hard
way.

---

## Globals are not slots either

**Added 2026-09-07, for §statements' top-level bindings.** Decision 5 makes every local a
function-scoped slot, and a slot cannot outlive its frame. A top-level binding has to, so
`Program` carries a `globals` table beside `funcs`, and the IR gains `Rvalue::GlobalGet`
and `Place::Global` — a read and a write, and no third instruction, exactly as cells did.

**They are the reason a `fn` can read a top-level binding at all.** §functions promises a
`fn` carries no environment, and reaching a *slot* of an enclosing frame would break it.
Reaching a global does not: it is a location, so the read is one `global.get` on wasm and
one index into a table in the interpreter. Slots map onto wasm locals, globals onto wasm
globals; §scope predicted that distinction would matter here and this is where it lands.

A global needs no cell. A cell exists to give a captured binding a home outliving its
frame, and a global already has one — so the capture analysis above simply never sees one.

**Initialization is checked, not defaulted.** A global's `let` lowers to a `Store` at the
position it was written, so between the start of the entry function and that statement the
global holds nothing meaningful. Rather than invent a default — §types has no `nil` and
§statements promises no uninitialized state is observable — elaboration computes the
globals each function may read, following direct calls to a fixed point and answering an
indirect call with the union over every function whose value is taken, and refuses a
top-level call that could reach one that has not been stored yet. The IR therefore carries
no initialization flag and neither back end emits a check: the guarantee is discharged
before it gets here.

---

## Shape

**The crate is the normative statement.** `ir/src/program.rs` holds the instruction set and
the three invariants above as doc comments; `ir/src/types.rs` and `ir/src/consts.rs` hold
the type table and the constant pool. An abridged copy here would drift, and it did — the
sketch that used to sit in this section predated the `Operand` refinement and still carried
a `Cast { to: TypeId }`, which invariant 1 forbids.

What the reader should know without opening the crate:

- A `Stmt` is `Assign`, `Store`, `If`, `While`, `Break`, `Continue`, `Return`, or `Drop`.
  Nothing else, and nothing that jumps.
- An `Rvalue` is `Use`, `Unary`, `Binary`, `Cast`, `Call`, `CallIndirect`, `MakeStruct`,
  `Field`, `MakeCell`, `CellGet`, or `MakeClosure`.
- An `Operand` is a `Slot` or a `ConstId`. Invariant 2 is that rule, and it is the reason
  the list above has no nested-expression variant.
- A `Place` — the target of a `Store` — is a field or a cell. Assigning a plain local is
  an `Assign`, so `Place` carries only the forms `Assign` cannot express.

`CallIndirect` exists from the first day even though §objects is unstarted, because
lambdas need it already (§functions: `funcref` in a table, `call_indirect`) and vtables
will be the same node when §objects arrives.

### What the IR deliberately lacks

The rule the crate follows: **it contains only what lowering can produce**, so there is no
node that cannot be exercised. Four things this document anticipated are therefore absent,
each additive and each blocked on a section rather than on a decision:

| Absent | Waiting on |
| --- | --- |
| An instantiation chain on provenance; `Instantiation`, `InstId` | §comptime, once `comptime` has a token. Provenance is one CST node today. |
| `CallHost`, `HostId` | §modules, once a program can declare what it needs from the host. Until then a program can compute but cannot observably *do* anything (§statements). |
| `Index`, and every collection type | §types and §generics. `Str` is the only indexable thing, and what indexing it returns is open. |
| `Trap` | §errors. Constant failures are diagnostics (§expressions), and nothing else traps at lowering time yet. |

### The type table

Two kinds of entry, because §types is nominal:

- **Nominal** — a struct. Identity is the *allocation site*: §types says every evaluation
  of a `struct { … }` expression produces a fresh type, so every evaluation allocates a
  fresh `TypeId`. `Pair(u64, Str)` is one type because §comptime's instantiation cache
  runs the body once, not because two structurally identical structs are interned. **They
  must not be.**
- **Structural** — primitives, unit, and `fn(T, …) -> R`. These have no identity and are
  interned normally.

### Provenance

Statements carry a `CstId`. Functions carry an `InstantiationId` naming the chain of
comptime call sites that produced them, so §comptime's requirement is met: a diagnostic
about an instantiation can name real source in both the generic body and the call that
instantiated it.

### Crate layout

**Split 2026-09-07.** `ir` used to hold the representation, elaboration, and the dump
together. It now holds representation and the dump only:

```
tokenizer ← parser ← ir      (program, types, consts, syms, print)
                     ir ← eval   (executes a Program)
              ir, eval ← elab   (produces a Program; runs eval for comptime)
                         elab ← interpreter, lsp, wasm
```

Elaboration and evaluation are mutually recursive (§comptime), so elaboration cannot live
in the crate the evaluator depends on. The split was made *before* `comptime` has a token,
because it is the shape the feature lands on rather than part of it: `elab` declares its
dependency on `eval` and does not yet call it.

Two consequences of the boundary, both of which fell out rather than being designed:

- **`ir` no longer depends on `tokenizer`.** Nothing in the representation talks about
  source text. Spans belong to diagnostics, and diagnostics belong to elaboration.
- **A failure splits at the same seam as everything else.** `eval::Trap` carries a
  `CstId`, which is core IR's provenance and all the executor has;
  `interpreter::RuntimeError` carries a `Span`, and the conversion happens in the one
  place still holding the tree. The doc comment describing that division predated the
  split and now describes a crate boundary rather than two types in one file.

The wasm back end does not depend on the evaluator: by the time it runs, elaboration has
already folded every comptime expression.

---

## What core IR does not contain

A closed list, because every entry is a place the two back ends could otherwise disagree.

- **Generics, `comptime`, and `Type`** — §comptime; resolved by elaboration.
- **Names** — slots and indices only. Name resolution (§scope) has happened.
- **Nested expressions** — decision 1.
- **Implicit conversion and truthiness** — §expressions has neither; every `Cast` is
  explicit and every condition is already `bool`.
- **Overloading** — §functions has none.
- **`goto`, or unstructured edges of any kind** — §control declined it and wasm has none.
- **Layout, `sizeof`, pointers** — §types says layout is not user-visible; the IR talks
  about types, and the back ends decide representation.
- **`elif`, compound assignment, and the rest of §statements' sugar** — desugared.

---

## Conformance

Goal §both-modes asks for a shared conformance suite "from the first day there are two
back ends." It attaches here: a program is elaborated once to core IR, then executed by
the interpreter and by compiled wasm, and the observable results must be identical.

Decision 2 makes the interpreter the reference semantics in a stronger sense than that —
it is also what computed every comptime value the wasm build contains.

**Done, 2026-08-19.** This used to say that the CST interpreter then under construction
was a bring-up artifact sitting at the wrong end of the pipeline, and that it should take
core IR as its input before there was a second back end to disagree with. It does. The
tree walker was retired the day after this document was written, and the semantics that
had accumulated in it — name resolution, coercion, evaluation order — moved into
elaboration, which is the only place they can be stated once for both back ends.

`eval` takes a `Program` and nothing else: no tree, no names, no source. That is the
property the conformance suite needs, and since the crate split it is enforced by the
crate graph rather than by intention — `eval` does not declare `parser`, so a `Tree` is
not a thing it can name. The one type that reaches it from there is `CstId`, which is a
`NodeId` it carries on a trap and never looks inside. There is no tree for a semantics to
accumulate against.

**Nothing is being conformed yet.** There is one back end, and the suite this section
describes attaches the day there are two.

---

## Open

- **Async and suspension.** ANF makes the live-state question answerable, but nothing here
  decides how a suspension point is *represented*. It comes back with §control and
  §modules.
- **`comptime var` and `inline for`** (§comptime's open items). Decision 2 makes both
  nearly free — the evaluator already has mutation and already has loops — which is an
  argument for granting them, not a decision to.
- **How comptime rejects its arguments** (§comptime). With one evaluator this is an abort
  that becomes a diagnostic in one mode and a trap in the other; what the *source* spells
  to invoke it is still §comptime's.
- **Static materialization fast path** for flat acyclic comptime constants (decision 3).
- **Pattern matching** (§unions) has no lowering here yet. Decision trees versus
  `br_table` is §unions', but the IR node it produces is this document's, and neither
  exists.
- **Whether `Drop` should require unit.** §statements says nothing marks a discard as
  deliberate; if that changes, it changes here.

## Related

- `design-goals.md` — §both-modes (two back ends, one semantics), §liveness (liveness vs.
  wasm)
- `constructs/14-metaprogramming-and-tooling.md` — comptime, elaboration, and the sentence
  this document expands
- `constructs/04-control-flow.md` — structured control flow, and why there is no CFG here
- `constructs/06-data-and-types.md` — reference semantics, GC, type identity
- `constructs/05-functions.md` — closures, capture, and the per-iteration promise
- `constructs/15-invisible-semantics.md` — the divergence register ANF is aimed at
