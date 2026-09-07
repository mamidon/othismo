# §unions — Discriminated Unions & Pattern Matching

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox omits both, and they are one feature rather than two: a sum type you can't destructure
is inert, and a `match` with nothing to match on is a `switch`. Table stakes for a modern
language, and cheap in novelty budget — the spellings are well-established (`enum` +
`match`, or `type` + `case`), so goal §boring says pick a conventional one and move on.

The payoff is **exhaustiveness checking**. Everything else — the terseness, the
destructuring — is convenience; the compiler telling you that you forgot a case is the
reason to have the feature. That in turn makes this the natural foundation for `Option`
and `Result`, which is why error handling (§errors) comes after this section and generics
(§generics) rather than before them.

Three things to watch:

- **wasm.** No native variant type. A discriminated union is a tag plus a payload, and its
  layout is an ABI decision (§types, §wasm). A dense integer match compiles to `br_table`
  cheaply; a match on structure does not.
- **Openness and versioning.** A closed enum gives exhaustiveness; an open one survives
  a module or a message format gaining a case. Othismo's BSON messages will gain cases.
  Both can't be true at once, and the usual escape (a mandatory catch-all) throws away
  the entire benefit.
- **Dynamic values.** Matching against a BSON document arriving from the host isn't
  matching a statically-known union. Whether that's the same construct, or pattern
  matching plus a runtime type test, is a real decision.

## Checklist

- **Union declaration**
  - Nominal enums with payloads vs. structural/anonymous unions
  - Payload forms: none, positional/tuple, named/record
  - Generic unions (§generics) — `Option[T]`, `Result[T, E]`
  - Recursive unions, and what they imply for the memory model (§types)
- **Construction** — bare constructor names vs. qualified (`Some(x)` vs. `Option.Some(x)`);
  whether constructors are first-class functions
- **Representation** **[wasm]** — tag encoding, payload layout, niche/pointer-tagging
  optimizations, and whether representation is guaranteed or unspecified
- **Patterns**
  - Literal, wildcard `_`, variable binding
  - Constructor patterns with payload destructuring
  - Tuple / record / array patterns; rest patterns
  - Or-patterns (`A | B`), guards (`if cond`), range patterns
  - Nested patterns, and binding a whole value alongside its parts (`x @ Some(_)`)
  - Type-test patterns, for dynamic values and for §inference's `dynamic` escape hatch
- **Exhaustiveness and redundancy** — the reason the feature exists
  - Static exhaustiveness checking; how errors are reported
  - Unreachable-arm detection
  - The catch-all arm as an exhaustiveness defeater — allowed, discouraged, or lint
- **Openness / versioning** — non-exhaustive or extensible unions across a module
  (§modules) or message (§errors) boundary, and what that costs in checking
- **`match` as expression vs. statement** (§statements) — and whether all arms must agree
  in type
- **Irrefutable patterns elsewhere** — in `let` bindings, function parameters
  (§functions), `for-in` loop variables (§control)
- **Refutable pattern sugar** — `if let` / `while let`, or whatever the equivalent is
- **`switch` with fallthrough** — the worse version of this; presumably not, but decide
- **Matching dynamic / host data** — BSON documents, `dynamic` values, instance replies:
  same construct or a different one?
- **Compilation** **[wasm]** — decision trees vs. `br_table`; how much of the checking
  the interpreter tier duplicates (goal §both-modes)

## Glue Syntax

## Glue Semantics
