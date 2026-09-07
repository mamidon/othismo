# §generics — Generics & Polymorphism

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox omits generics because it's dynamically typed — a dynamic language gets polymorphism
for free and pays for it at every call site. Glue's optional annotations (§inference) mean
it can't dodge the question: as soon as a type can be written down, someone will want to
write down "a list of these."

The central decision was **monomorphization vs. boxing**, and it looked awkward under goal
§both-modes, because the two execution modes appeared to want opposite answers: the wasm
compiler wants a specialized copy per instantiation, unboxed and fast; an interpreter
usually wants erasure or boxing, one implementation with values carrying their own type.
Picking one per back end is fine *as long as the semantics are identical*, and erasure
versus monomorphization is exactly the split that breaks that.

**It is no longer a choice.** §comptime makes types comptime values, so a generic type
does not exist until it is instantiated and there is nothing left to erase. Both back ends
consume the same monomorphic core IR — which is goal §both-modes' shared front end
arriving as a consequence rather than as discipline, since two tiers reading one IR cannot
disagree about which programs they accept.

What that costs, stated where it is paid: the interpreter no longer starts instantly on
generic code. It elaborates and instantiates first, memoized on `(declaration, comptime
arguments)`. For a prompt line that is a handful of instantiations; the claim in goal
§both-modes is instant *startup*, not zero work per line, and this does not threaten it.
It would be worth revisiting if a REPL session were ever observed paying it repeatedly.

The remaining code-size question is real and unchanged: monomorphization costs module
size, and in an Othismo image a module is something you carry around and hot-replace, not
just something you download once.

Second decision: how a generic parameter is **constrained**. Unconstrained parameters can
only be moved around; useful generics need bounds, which pulls in interfaces/traits from
§objects and connects to whether `+` on a `T` is even expressible.

The Othismo angle: a message handler's parameters arrive as BSON. Whether a handler can be
generic at all — and what a type parameter would even mean once the value has been
serialized and routed — is an open question that §modules and §errors both touch.

**Decided elsewhere, 2026-08-18.** [§comptime](14-metaprogramming-and-tooling.md) settles
the mechanism: types are comptime values, a generic is a function taking or returning one,
and monomorphization is memoized function application. §generics therefore has **no syntax
of its own** — an instantiation is a call, which §expressions' postfix rung already
parses. What remains here is everything that isn't a spelling: bounds, variance, inference
of type arguments, and generic message handlers. The cost §comptime accepts on §generics'
behalf is that a generic body is type-checked only when instantiated, so an uninstantiated
generic gets parsing and name resolution and nothing more.

## Status

Legend in the [index](../language-constructs.md). *Syntax* and *Semantics* track what has
been **decided**; *Implementation* tracks what is **built** in `glue/`.

| Area | Syntax | Semantics | Implementation |
| --- | --- | --- | --- |
| Type parameters — a generic is a function over `Type` | ✓ | ✓ | — |
| Instantiation — an instantiation is a call | ✓ | ✓ | — |
| Monomorphization, memoized on `(declaration, arguments)` | · | ✓ | — |
| Bounds and constraints | — | — | — |
| Variance | — | — | — |
| Inference of type arguments at a call site | — | — | — |
| Generic collections | — | — | — |
| Generic message handlers | — | — | — |
| Checking an uninstantiated generic | — | — | — |

The first three rows were decided in §comptime rather than here, which is why this
section has no syntax of its own. None of it is implemented: `comptime` has no token, so
there is no `Type`, no instantiation, and no cache.

---

## Checklist

- **Generic functions** — declaration syntax, parameter list placement, and how they read
  when annotations are otherwise optional
- **Generic types** — over structs, unions (§unions), and interfaces; multiple parameters
- **Constraints / bounds**
  - Interfaces / traits / typeclasses as bounds (§objects), or structural constraints
  - Operator constraints — can a generic function use `+` on its parameter?
  - Multiple bounds; where-clauses; defaults for type parameters
  - Associated types vs. extra type parameters
- **Inference of type arguments** at the call site (§inference), and the
  explicit-instantiation syntax for when it fails
- **Variance** — covariance/contravariance for generic containers, or invariance only
- **Higher-kinded types** — almost certainly not, but it's a spend worth declining on
  purpose
- **Implementation strategy** **[wasm]**
  - ~~Monomorphize vs. box/erase, and whether the two back ends may differ~~ —
    **answered by §comptime:** monomorphize, and they may not differ
  - Code-size cost of monomorphization against image and module size (goal §liveness)
  - Specialization of already-generic code; instantiation across module boundaries
    (§modules) and what that means for separate compilation
  - Whether type arguments are reified at runtime — required if reflection (§comptime) or
    introspection (goal §living) is expected to see them
- **Dynamic dispatch as the alternative** — trait objects / existentials, vtables and
  `call_indirect` (§wasm); when the language picks one for you
- **Interaction with dynamic values** — what happens when an unannotated or `dynamic`
  value (§inference) meets a generic signature; where the check happens and who's to blame
- **Generic message handlers** — Othismo-specific: can an operation be generic when its
  arguments are BSON on the wire? (§errors, §modules)
- ~~**Monomorphization and the interpreter**~~ — **answered by §comptime:** the
  interpreter does specialize, over the same core IR, so the question of whether it
  rejects the same programs cannot arise (goal §both-modes conformance suite)

## Glue Syntax

## Glue Semantics
