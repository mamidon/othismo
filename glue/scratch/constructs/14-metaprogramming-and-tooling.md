# §14 — Metaprogramming & Tooling Constructs

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Not core language, but genuinely constructs — and painful to retrofit, because each one
either needs a syntax slot reserved for it or a compiler pipeline shaped to accommodate
it.

Three have specific pull for Glue. Goal §2.2 (cheap to compile *and* cheap to interpret)
argues against an elaborate macro layer, since that is one of the classic ways a language
becomes miserable to compile. Goal §2.4 (inspect a running system) needs *something* at
runtime describing program structure, which is reflection under another name. And the
same goal makes test declarations more than good hygiene: an interpreter and a compiler
must be held to a shared conformance suite from the day there are two back ends.

## Checklist

- **Macros** — textual, syntactic, or hygienic; or none
- **Compile-time evaluation** (`const fn`, comptime)
- **Reflection** / runtime type info
- **Annotations / attributes / decorators** — including the ones the compiler consumes
  (`#[export]`, `#[inline]`)
- **Conditional compilation** / feature flags
- **`assert` / contracts** as syntax vs. library
- **Doc comments** and doc generation
- **Source maps / debug info** **[wasm]** — DWARF in wasm, name section
- **Test declarations** (see Design Note: Test Your Language, ch. 14 — the book is
  emphatic about this)

## Glue Syntax

## Glue Semantics
