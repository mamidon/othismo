# §6 — Data & Types

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox is dynamically typed with four primitives: bool, number (f64), string, nil (see
Design Note: Static and Dynamic Typing, ch. 7). Almost everything else in this section is
something Lox leaves out, and almost all of it interacts with the wasm target.

This section covers the *data* — what values exist and how they're represented. Three
neighbouring concerns have their own sections because each is large enough to design
independently: discriminated unions (§7), generics (§8), and inference and gradual typing
(§10). What stays here is scalars, aggregates, the nominal/structural question, value vs.
reference semantics, and memory.

Goal §4.1 pushes hardest: if liveness means meaningful state lives in the Othismo
namespace rather than in linear memory, that changes what the data model should even be.
And memory is the part wasm will not let you defer — GC, allocation, layout, and string
representation all leak into the host ABI whether or not you intend them to.

## Checklist

- **Primitives** — bool, number (f64), string, nil **[Lox]**
- **Integers** as a distinct type **[Lox-omits]** **[wasm]** — with wasm you have i32/i64
  natively; a single f64 number type is a real cost. Decide: widths, signedness, literals,
  conversions, overflow (trap or wrap — §9, §15)
- **Aggregates** **[Lox-omits]**
  - Arrays / fixed-size vs. growable lists
  - Tuples
  - Records / structs (nominal vs. structural)
  - Maps / dictionaries (ch. 20 builds hash tables for the implementation, but Lox
    doesn't expose them)
  - Sets
- **Sum types / enums / tagged unions** **[Lox-omits]** — see §7, with pattern matching
- **Optional / nullable types** — or: no `nil` at all, which removes a whole error class.
  If §7 lands, this is `Option[T]` rather than a language feature
- **Type aliases**, newtypes, opaque types
- **Generic / parametric types** — see §8
- **Interfaces / traits / protocols / typeclasses** — declared here or in §11; used as
  generic bounds in §8
- **Value semantics vs. reference semantics** — per type or per usage
- **Type inference** — see §10
- **Memory model** **[wasm]**
  - GC vs. reference counting vs. ownership/borrowing vs. arenas (ch. 26 does mark-sweep)
  - wasm linear memory: who allocates? Is there an `alloc` in the language or a library?
  - wasm GC proposal vs. rolling your own in linear memory
  - Pointers/references as a user-visible construct at all
- **Layout & representation** **[wasm]** — struct layout, alignment, endianness,
  string representation, union tag/payload layout (§7). These leak into the ABI whether
  you want them to or not
- **Serialization to BSON** — Othismo-specific: which types can cross a message boundary,
  and whether that's a property of the type or of the send (§13, §9)

## Glue Syntax

## Glue Semantics
