# §wasm — Cross-Cutting: wasm Target Decisions

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Not a feature area — a set of constraints from the compilation target that cut across
every section above. Collected here because each one forces a decision somewhere else,
and because they are the reason several of Lox's omissions stop being optional.

Behind all of them sits goal §liveness: wasm modules are immutable once instantiated, and
"reach into a deployed system and change it" is very nearly the one thing the target does
not want to allow. Whatever resolves that — state in the namespace rather than linear
memory, module replacement with state migration, an interpreter tier inside a running
instance, or some combination — will reach back into the memory model, the module system,
and what "an object" even means.

## Checklist

| Decision | Why it's forced | Lands in |
| --- | --- | --- |
| Number types | wasm has i32/i64/f32/f64; "just use f64" (Lox) is a real cost | §lexical, §types, §inference |
| Memory management | No GC without the GC proposal; otherwise write your own in linear memory | §types |
| Closures | No native support; needs environment objects + function tables | §functions |
| Dynamic dispatch | `call_indirect` + type-indexed tables | §functions, §generics, §objects |
| Exceptions | Proposal-level; consider Result types instead | §errors |
| Tail calls | Proposal-level; otherwise trampolines | §functions |
| Strings | Not a wasm type; you own the representation and the host boundary | §lexical, §types |
| Host interop | Imports/exports, and how source declarations map to them | §functions, §modules |
| ABI | Struct layout, calling convention, ownership across the boundary | §types, §unions |
| Structured control flow | wasm has no goto — nudges the language toward structured CF | §control |
| Multi-value returns | Supported by the multi-value proposal; enables tuples cheaply | §control, §types |
| Union representation | No variant type; a tag plus a payload, and its layout is an ABI decision | §unions |
| Monomorphization | Specializing generics costs module size, which an image carries and replaces | §generics |
| Boxing | Unannotated values need a uniform representation; this is §cliff, made concrete | §inference |
| Module linking | Static linking vs. late binding decides whether a live module can be replaced | §scope, §modules |
| Component model / WIT | Whether glue targets core wasm or components changes the interface language entirely | §functions, §modules |

## Glue Syntax

## Glue Semantics
