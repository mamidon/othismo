# §10 — Type Inference & Gradual Typing

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

This section is the machinery behind goal §2.1's promise that "type annotations are
optional wherever the compiler can manage without them." That promise is the whole reason
a one-liner and a module can be the same language — and it's a bigger commitment than it
looks, because there are two distinct ideas hiding in it:

- **Inference** — the type exists and the compiler works it out. Nothing is dynamic; you
  just didn't have to say it.
- **Gradual typing** — some values genuinely have no static type, and checks happen at
  runtime.

They have different costs and different failure modes, and a language can want both: full
inference in compiled code, a `dynamic` escape hatch for values arriving from the host as
BSON (§9, §13). What's not negotiable is being clear about which one is happening where.

Goal §2.2 constrains the inference algorithm directly. Whole-program Hindley–Milner is one
of the named ways a language becomes expensive to compile, and it fits badly with a REPL
and with separate compilation. Local or bidirectional inference — annotations required at
declaration boundaries, inferred everywhere inside — is the cheaper answer and the one
most modern languages have converged on. Goal §2.3 agrees: it's the conventional spelling.

Goal §4.4 is the tension to resolve here rather than discover later: unannotated code
usually means boxed, dynamically dispatched values, so there's a **performance cliff at the
annotation boundary**, plus a soundness question about whether annotations are trusted or
checked. Neither is fatal. Both are much cheaper to decide now.

## Checklist

- **Inference scope**
  - Local / bidirectional vs. whole-program Hindley–Milner (goal §2.2 argues for local)
  - Where annotations are *required*: function signatures, exports (§13), message
    handlers, recursive definitions, top-level bindings
  - Inference for generic type arguments (§8) and for union constructors (§7)
- **Defaulting rules** — what an unannotated numeric literal becomes **[wasm]**. With
  `i32/i64/f32/f64` on the target and no single "number" type (§1, §6), literal defaulting
  is a user-visible decision, not a formality
- **Dynamic typing**
  - Is there a `dynamic` / `any` type, and where is it allowed?
  - Implicit at unannotated positions, or only when written explicitly?
  - Host data (BSON) as the motivating case: it arrives untyped and must be narrowed —
    presumably by pattern matching with type tests (§7)
- **Soundness and blame**
  - Are annotations trusted (erased, fast, unsound at the boundary) or checked (casts
    inserted at the dynamic/static boundary, sound, slower)?
  - Where a mixed-typing failure is reported, and whether it names the right code
- **Representation and the cliff** **[wasm]**
  - Boxed dynamic values vs. unboxed typed values; uniform representation as the
    alternative
  - Whether the cliff is *visible* — can a user tell which code is fast, and are they
    told, or do they have to measure?
- **Diagnostics** — inference makes errors appear far from the mistake; error quality is
  a design constraint, not a polish item
- **Interactive and incremental typing**
  - Inference at a prompt, one definition at a time, without the whole program
  - Redefining a binding to a new type in a live image (goal §4.5)
  - Whether the interpreter tier type-checks at all, and if not, whether it accepts
    programs the compiler rejects (goal §2.2 — the two must not diverge)
- **Separate compilation** — inferred types crossing a module boundary (§13); whether
  module interfaces are inferred or must be written
- **Runtime type information** — erasure vs. reified types, which goal §2.4's
  introspection may require regardless of what the type checker needs (§14)

## Glue Syntax

## Glue Semantics
