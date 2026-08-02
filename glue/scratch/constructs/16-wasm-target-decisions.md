# §16 — Cross-Cutting: wasm Target Decisions

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Not a feature area — a set of constraints from the compilation target that cut across
every section above. Collected here because each one forces a decision somewhere else,
and because they are the reason several of Lox's omissions stop being optional.

Behind all of them sits goal §4.1: wasm modules are immutable once instantiated, and
"reach into a deployed system and change it" is very nearly the one thing the target does
not want to allow. Whatever resolves that — state in the namespace rather than linear
memory, module replacement with state migration, an interpreter tier inside a running
instance, or some combination — will reach back into the memory model, the module system,
and what "an object" even means.

## Checklist

| Decision | Why it's forced | Lands in |
| --- | --- | --- |
| Number types | wasm has i32/i64/f32/f64; "just use f64" (Lox) is a real cost | §1, §6, §10 |
| Memory management | No GC without the GC proposal; otherwise write your own in linear memory | §6 |
| Closures | No native support; needs environment objects + function tables | §5 |
| Dynamic dispatch | `call_indirect` + type-indexed tables | §5, §8, §11 |
| Exceptions | Proposal-level; consider Result types instead | §9 |
| Tail calls | Proposal-level; otherwise trampolines | §5 |
| Strings | Not a wasm type; you own the representation and the host boundary | §1, §6 |
| Host interop | Imports/exports, and how source declarations map to them | §5, §13 |
| ABI | Struct layout, calling convention, ownership across the boundary | §6, §7 |
| Structured control flow | wasm has no goto — nudges the language toward structured CF | §4 |
| Multi-value returns | Supported by the multi-value proposal; enables tuples cheaply | §4, §6 |
| Union representation | No variant type; a tag plus a payload, and its layout is an ABI decision | §7 |
| Monomorphization | Specializing generics costs module size, which an image carries and replaces | §8 |
| Boxing | Unannotated values need a uniform representation; this is the §4.4 cliff, made concrete | §10 |
| Module linking | Static linking vs. late binding decides whether a live module can be replaced | §12, §13 |
| Component model / WIT | Whether glue targets core wasm or components changes the interface language entirely | §5, §13 |

## Glue Syntax

## Glue Semantics
