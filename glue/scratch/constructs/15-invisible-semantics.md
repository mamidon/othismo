# §semantics — Semantics You Must Decide Even If There's No Syntax For Them

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

The invisible half of a language spec: decisions with no surface syntax that are
nonetheless part of the language, and that two implementations must agree on. Goal
§both-modes makes this section load-bearing — an interpreter and a wasm compiler diverge
exactly here, quietly, unless the answers are written down and tested.

wasm is strict IEEE-754 and largely deterministic; that is a property worth not
accidentally giving away. And Othismo's messages are re-entrant and interleaved, so "what
happens if a handler is entered again while it is suspended" is a language-level answer,
not a runtime footnote.

Note that this file's **Glue Semantics** section is the substantive one; **Glue Syntax**
will mostly stay empty by construction, and where it doesn't, that's a sign the decision
actually belongs in another section.

## Status

Legend in the [index](../language-constructs.md). *Syntax* and *Semantics* track what has
been **decided**; *Implementation* tracks what is **built** in `glue/`.

| Area | Syntax | Semantics | Implementation |
| --- | --- | --- | --- |
| Evaluation order — left to right, specified | · | ✓ | ✓ |
| Short-circuiting | · | ✓ | ✓ |
| Coercion — none, and no truthiness | · | ✓ | ✓ |
| Integer overflow and division by zero | · | ✓ | ✓ |
| NaN, ±0, and float determinism | · | ✓ | ✓ |
| Mutability and aliasing | · | ✓ | ✓ |
| Identity versus equality | · | ◑ | ◑ |
| String encoding and indexing | · | ◑ | ◑ |
| Trap taxonomy — what belongs in each bucket | · | ◑ | ◑ |
| A total-order companion to IEEE `==` | · | — | — |
| Resource limits — stack depth, memory growth | · | ◑ | ◑ |
| Initialization order of globals and modules | · | — | — |
| Reentrancy and the message model | · | — | — |

The **Syntax** column is `·` throughout by construction — that is what this section is.
Most of what it owns was in fact decided in §expressions, §types, and §functions and is
implemented; this table is the first place they are collected. Identity-versus-equality is
◑ because §types and the interpreter currently disagree; string indexing is ◑ because the
encoding is decided and indexing is not implemented; resource limits are ◑ because the
recursion limit exists and memory growth has no answer.

---

## Checklist

- **Evaluation order** of operands and arguments (left-to-right, or unspecified)
- **Short-circuit** behavior — already listed in §expressions, but it's semantics, not
  syntax
- **Coercion rules** — implicit numeric widening, string coercion (`"a" + 1`)
- **Mutability & aliasing** — can two names see the same mutable object
- **Identity vs. equality** for each type
- **Initialization order** of globals/modules
- **Integer overflow, division by zero, NaN, ±0, float determinism** **[wasm]** — wasm is
  strict IEEE-754 and mostly deterministic; don't accidentally give that up
- **String encoding and indexing** — bytes, code points, or graphemes (Design Note, ch. 19)
- **Error/trap behavior** — what's a compile error, runtime error, trap, or UB. Ideally:
  no UB. The taxonomy itself is §errors; what belongs in each bucket is decided here
- **Resource limits** — stack depth, memory growth, message size **[wasm]** — memory
  growth and traps are observable
- **Concurrency / memory model** — even single-threaded, define reentrancy semantics.
  othismo's messages are re-entrant and interleaved, which the language must account for

## Glue Syntax

## Glue Semantics
