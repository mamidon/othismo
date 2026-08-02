# §8 — Generics & Polymorphism

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox omits generics because it's dynamically typed — a dynamic language gets polymorphism
for free and pays for it at every call site. Glue's optional annotations (§10) mean it
can't dodge the question: as soon as a type can be written down, someone will want to
write down "a list of these."

The central decision is **monomorphization vs. boxing**, and goal §2.2 makes it awkward,
because the two execution modes want opposite answers:

- The **wasm compiler** wants monomorphization — a specialized copy per instantiation,
  unboxed and fast. The cost is code size, and in an Othismo image, module size is
  something you carry around and hot-replace, not just something you download once.
- The **interpreter** wants erasure or boxing — one implementation, values carrying their
  own type. Instant startup, no specialization pass.

Both are legitimate; picking one per back end is fine *as long as the semantics are
identical*, which is exactly what erasure vs. monomorphization tends to break
(specialization, reified type parameters, and anything reflective). Deciding this early
is what keeps the two tiers honest.

Second decision: how a generic parameter is **constrained**. Unconstrained parameters can
only be moved around; useful generics need bounds, which pulls in interfaces/traits from
§11 and connects to whether `+` on a `T` is even expressible.

The Othismo angle: a message handler's parameters arrive as BSON. Whether a handler can be
generic at all — and what a type parameter would even mean once the value has been
serialized and routed — is an open question that §13 and §9 both touch.

## Checklist

- **Generic functions** — declaration syntax, parameter list placement, and how they read
  when annotations are otherwise optional
- **Generic types** — over structs, unions (§7), and interfaces; multiple parameters
- **Constraints / bounds**
  - Interfaces / traits / typeclasses as bounds (§11), or structural constraints
  - Operator constraints — can a generic function use `+` on its parameter?
  - Multiple bounds; where-clauses; defaults for type parameters
  - Associated types vs. extra type parameters
- **Inference of type arguments** at the call site (§10), and the explicit-instantiation
  syntax for when it fails
- **Variance** — covariance/contravariance for generic containers, or invariance only
- **Higher-kinded types** — almost certainly not, but it's a spend worth declining on
  purpose
- **Implementation strategy** **[wasm]**
  - Monomorphize vs. box/erase, and whether the two back ends may differ
  - Code-size cost of monomorphization against image and module size (goal §4.1)
  - Specialization of already-generic code; instantiation across module boundaries (§13)
    and what that means for separate compilation
  - Whether type arguments are reified at runtime — required if reflection (§14) or
    introspection (goal §2.4) is expected to see them
- **Dynamic dispatch as the alternative** — trait objects / existentials, vtables and
  `call_indirect` (§16); when the language picks one for you
- **Interaction with dynamic values** — what happens when an unannotated or `dynamic`
  value (§10) meets a generic signature; where the check happens and who's to blame
- **Generic message handlers** — Othismo-specific: can an operation be generic when its
  arguments are BSON on the wire? (§9, §13)
- **Monomorphization and the interpreter** — if the interpreter doesn't specialize, does
  it still reject the same programs? (goal §2.2 conformance suite)

## Glue Syntax

## Glue Semantics
