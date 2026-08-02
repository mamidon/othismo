# §12 — Scope & Name Resolution

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

How a name finds its binding. The larger question of how source is organized into units —
modules, imports, entry points — is §13, because for Glue that's a design problem rather
than a checklist item.

Lox scopes lexically with block scope, makes use-before-declaration in the same scope a
static error, and resolves closure capture in a separate static pass. It late-binds
globals and slot-resolves locals — a distinction that gets sharper on wasm, where locals
become wasm locals and globals become wasm globals or memory.

Late binding is worth noticing here rather than only in §13: it's slower and less
checkable, and it's also the mechanism that makes redefining something in a live image
possible at all (goal §4.1, §4.5). Which names are late-bound is therefore a liveness
decision wearing a scoping decision's clothes.

## Checklist

- **Lexical scoping** **[Lox]**, ch. 11 — resolving and binding
  - Block scope **[Lox]** vs. function scope vs. hoisting
  - The "use before declaration in same scope" rule (Lox makes it a static error)
  - Closure capture resolution as a separate static pass (§5)
- **Globals** **[Lox]** — late-bound in Lox (ch. 21) vs. locals (ch. 22) which are
  slot-resolved. The distinction matters for wasm: locals → wasm locals, globals →
  wasm globals or memory
- **Late binding as a liveness mechanism** — which references are resolved at compile
  time and which stay indirect, and what that costs (goal §4.1)
- **Shadowing** — within a scope, across nested scopes, and across imports (§13)
- **Name resolution order** — locals, enclosing scopes, module scope, imports, prelude
- **Namespacing of member names** — do fields and methods share a namespace (Lox: yes,
  §11)? Do types and values? Do operations on an instance?
- **Bindings in patterns** — what a pattern match introduces, and its scope (§7)
- **Redefinition in a live session** — rebinding a name at a prompt or in a running image,
  including to a different type (§10, goal §4.5)

## Glue Syntax

## Glue Semantics
