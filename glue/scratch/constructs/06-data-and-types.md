# §types — Data & Types

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox is dynamically typed with four primitives: bool, number (f64), string, nil (see
Design Note: Static and Dynamic Typing, ch. 7). Almost everything else in this section is
something Lox leaves out, and almost all of it interacts with the wasm target.

This section covers the *data* — what values exist and how they're represented. Three
neighbouring concerns have their own sections because each is large enough to design
independently: discriminated unions (§unions), generics (§generics), and inference and
gradual typing (§inference). What stays here is scalars, aggregates, the
nominal/structural question, value vs. reference semantics, and memory.

Goal §liveness pushes hardest: if liveness means meaningful state lives in the Othismo
namespace rather than in linear memory, that changes what the data model should even be.
And memory is the part wasm will not let you defer — GC, allocation, layout, and string
representation all leak into the host ABI whether or not you intend them to.

## Checklist

- **Primitives** — bool, number (f64), string, nil **[Lox]**
- **Integers** as a distinct type **[Lox-omits]** **[wasm]** — with wasm you have i32/i64
  natively; a single f64 number type is a real cost. Decide: widths, signedness, literals,
  conversions, overflow (trap or wrap — §errors, §semantics)
- **Aggregates** **[Lox-omits]**
  - Arrays / fixed-size vs. growable lists
  - Tuples
  - Records / structs (nominal vs. structural)
  - Maps / dictionaries (ch. 20 builds hash tables for the implementation, but Lox
    doesn't expose them)
  - Sets
- **Sum types / enums / tagged unions** **[Lox-omits]** — see §unions, with pattern
  matching
- **Optional / nullable types** — or: no `nil` at all, which removes a whole error class.
  If §unions lands, this is `Option[T]` rather than a language feature
- **Type aliases**, newtypes, opaque types
- **Generic / parametric types** — see §generics
- **Interfaces / traits / protocols / typeclasses** — declared here or in §objects; used
  as generic bounds in §generics
- **Value semantics vs. reference semantics** — per type or per usage
- **Type inference** — see §inference
- **Memory model** **[wasm]**
  - GC vs. reference counting vs. ownership/borrowing vs. arenas (ch. 26 does mark-sweep)
  - wasm linear memory: who allocates? Is there an `alloc` in the language or a library?
  - wasm GC proposal vs. rolling your own in linear memory
  - Pointers/references as a user-visible construct at all
- **Layout & representation** **[wasm]** — struct layout, alignment, endianness, string
  representation, union tag/payload layout (§unions). These leak into the ABI whether you
  want them to or not
- **Serialization to BSON** — Othismo-specific: which types can cross a message boundary,
  and whether that's a property of the type or of the send (§modules, §errors)

## Glue Syntax

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Not decided here

- **Enums / tagged unions** — §unions. They are structs with a tag, and they share this
  section's semantics, but their declaration and matching are §unions'.
- **Generics** — §generics, with the mechanism in §comptime. This is why there are no
  collections yet: `List(T)` and `Map(K, V)` are generic types, so they can't precede
  them. §lexical's collection literals came out with them. What §comptime settles and this
  section uses is *Structs without a name*, below.
- **Traits, interfaces, and operator overloading** — §objects. Until then `+` and `==` are
  built-in and closed (§expressions).
- **Tuples** — not now. §lexical's `.5` lexing rule keeps `pair.0` available should they
  arrive.

### Primitive types

| Type | Notes |
| --- | --- |
| `bool` | `true` / `false`. The only thing a condition may be (§expressions, §control) |
| `u8 u16 u32 u64` | unsigned integers |
| `s8 s16 s32 s64` | signed integers |
| `f32 f64` | IEEE-754 binary floating point |
| `char` | one Unicode scalar value, 32 bits (§lexical) |
| `Str` | UTF-8 bytes, byte-indexed (§lexical) |
| `()` | unit — one value, one inhabitant |

Primitives have **value semantics**: assignment copies, and there is no way to observe
sharing. Their widths, literals, conversions, and trapping behavior are §lexical's and
§expressions'.

### Structs

```
struct Point {
  x: s64,
  y: s64,
}

let p = Point { x: 1, y: 2 };
p.x                                  // 1

let mut q = Point { x: 0, y: 0 };
q.x = 5;                             // permitted — q is mut
```

- **Nominal, not structural.** Two structs with identical fields are different types. A
  name is a decision, and structural typing makes every field name load-bearing forever.
  What gives an anonymous struct its identity is under *Semantics*.
- Field types are required; there is no inference across a declaration boundary
  (§functions, §inference).
- **Field mutability follows the binding**, not the field. There is no per-field `mut`:
  a `mut` binding permits assigning any field, a non-`mut` binding permits none. Per-field
  mutability is additive later and buys little before there's a reason for it.
- Field visibility is §modules', with modules.

### Structs without a name

**Added 2026-08-18, with §comptime.** `struct { … }` with the name left off is an
**expression** whose value is a type:

```
let Point = struct { x: s64, y: s64 };
```

§comptime needs it because a generic returns a type it cannot name in advance:

```
fn Pair(comptime A: Type, comptime B: Type) -> Type {
  struct { first: A, second: B }
}
```

The named form is then sugar, and so is the alias below:

```
struct Point { x: s64, y: s64 }     ≡   let Point = struct { x: s64, y: s64 };
type InstanceId = u64;              ≡   let InstanceId = u64;
```

Both spellings stay. They cost two sugar rules and they keep the declaration a reader
already recognizes, which is the trade goal §boring asks for — the novelty is that a type
is a value, and nobody who doesn't need that fact has to meet it. What it costs is the
`type` spelling: the keyword stays on the alias form, so the type of types is `Type`, a
predeclared name alongside `u64` and `Str` rather than a keyword.

**This raises the stakes on a question §statements already had.** If `struct Point { … }`
is a `let` statement, then two structs that refer to each other are two statements that
each need the other to have run — and §statements' rule is that statements run in order.
That question (how top-level declarations can be mutually recursive while statements run
in order) was already open and owned by §scope and §modules; what changes today is that
mutually recursive *types* are an ordinary thing to write, not an edge case, so the answer
can no longer be "declarations are special" without saying what a declaration is now that
it is sugar.

### Type aliases

```
type InstanceId = u64;
```

An alias is a second name for one type, not a new one — `InstanceId` and `u64` are
interchangeable everywhere. Since it is sugar for `let InstanceId = u64;`, that falls out
rather than being a rule: the two names are bound to one value. What the keyword still
buys is the assertion — `type X = …` requires its right-hand side to be a comptime-known
`Type`, and says so at the declaration instead of wherever `X` is first used as one.

A *distinct* type sharing a representation (a newtype) is a different feature and isn't
here yet; when §modules has visibility, it's worth revisiting together.

---

## Glue Semantics

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Reference semantics

**A struct is a reference.** Assigning one, passing it, or returning it copies a
reference, not the fields.

```
let a = Point { x: 1, y: 2 };
let mut b = a;
b.x = 99;
a.x                  // 99 — a and b are the same object
```

This settles three questions left open by earlier sections, and it settles them the
permissive way:

- **§statements' question — can a `mut` alias mutate what a non-`mut` binding observes?**
  Yes. So `let` means "you cannot mutate through *this name*", not "this value will not
  change". That's the weaker of the two readings, and the documentation must say the
  weaker one.
- **§functions' question — is a `mut` parameter by reference or copy-in/copy-out?** By
  reference. There is no copy to write back.
- **What `let` protects** is the binding and the fields reachable through it *by name*.
  Nothing more.

Aliasing bugs are therefore ordinary rather than exotic, which is the price of not
adopting either ownership (§non-goals rules it out) or copy-on-write value semantics (real
machinery in both back ends). It is the same bargain Java, Python, and JavaScript make.

**Opt-in value semantics — deliberately later.** Marking certain structs as copied on
assignment, the way Rust's `Copy` works, is the intended escape hatch for small
value-like types. It's additive: every program written under reference semantics keeps
working when a *new* marker appears. Doing it now would mean designing the marker, the
rules for which types may carry it, and its interaction with §objects' traits, all before
there's a program complaining.

### Equality

Structural, per §expressions: two structs are `==` when their fields are. Reference
identity — "the same object" — is a distinct question with no operator yet; §objects needs
one for instance references and can introduce it for both at once.

This is worth noticing as a genuine wart: under reference semantics, `==` compares
contents while assignment shares identity, so `a == b` does not imply `a` and `b` are
interchangeable, and mutating one changes the other. Every language in this family has it.

### Memory

- **Garbage collected.** No manual allocation, no `free`, no destructors, no ownership.
  §non-goals rules out being a systems language, and reference semantics plus manual
  freeing is the combination that produces use-after-free.
- **No finalizers and no weak references.** Both are hard to specify (order, timing,
  resurrection) and neither has a caller yet.
- **Cycles are collected**, which rules out naive reference counting as the *only*
  strategy. Whether the implementation is a tracing collector in linear memory or the wasm
  GC proposal is §wasm's, and the language deliberately exposes nothing that would let a
  program tell the difference.
- **No pointers, and no `sizeof` / `alignof`.** Layout is not user-visible. It still
  exists and still matters at the host boundary — §modules and §wasm own the ABI — but it
  isn't something a Glue program can ask about.

### Type identity

**Decided 2026-08-18, with §comptime.** Nominal typing needs a rule once types are values,
since `struct { … }` is now an expression that can be evaluated more than once.

**Every evaluation of a `struct { … }` expression produces a fresh type.** Two structs with
identical fields are different types, exactly as the nominal rule says, and this is the
same rule reached from the other side: identity comes from the act of construction, not
from the shape constructed.

That would make `Pair(u64, Str)` a different type from `Pair(u64, Str)` — which it is not,
because §comptime memoizes instantiation on `(declaration, comptime arguments)`. The body
runs once, so the `struct { … }` inside it is evaluated once, so there is one type.
**Nominal identity for generic types is a consequence of the instantiation cache**, and it
is the main thing that cache is load-bearing for beyond termination.

A type produced this way is named for diagnostics by the call that produced it —
`Pair(u64, Str)`, not `struct { first: u64, second: Str }`.

### What isn't here yet

Collections are the conspicuous absence: no lists, no maps, no arrays, no sets, and so no
`len()`, no indexing beyond `Str`, and no iteration (§control). All of it waits on
§generics, because all of it is generic. **Two questions come back with them:** what
integer type a length returns — `u64` is the presumptive answer since §lexical defaults
there, and anything else reintroduces mixed-sign arithmetic §expressions forbids — and
what happens when a collection is mutated while being iterated.
