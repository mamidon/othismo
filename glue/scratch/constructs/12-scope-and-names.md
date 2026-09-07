# §scope — Scope & Name Resolution

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

How a name finds its binding. The larger question of how source is organized into units —
modules, imports, entry points — is §modules, because for Glue that's a design problem
rather than a checklist item.

Lox scopes lexically with block scope, makes use-before-declaration in the same scope a
static error, and resolves closure capture in a separate static pass. It late-binds
globals and slot-resolves locals — a distinction that gets sharper on wasm, where locals
become wasm locals and globals become wasm globals or memory.

Late binding is worth noticing here rather than only in §modules: it's slower and less
checkable, and it's also the mechanism that makes redefining something in a live image
possible at all (goal §liveness, §image). Which names are late-bound is therefore a
liveness decision wearing a scoping decision's clothes.

## Status

Legend in the [index](../language-constructs.md). *Syntax* and *Semantics* track what has
been **decided**; *Implementation* tracks what is **built** in `glue/`.

| Area | Syntax | Semantics | Implementation |
| --- | --- | --- | --- |
| Lexical block scope | ✓ | ✓ | ✓ |
| Shadowing | ✓ | ✓ | ✓ |
| Use before declaration is a static error | · | ✓ | ✓ |
| Closure capture as a separate static pass | · | ✓ | ✓ |
| Name resolution order — locals, enclosing, prelude | — | ◑ | ◑ |
| Top-level order-independence and mutual recursion | ✓ | ✓ | ✓ |
| Globals versus locals | ✓ | ✓ | ✓ |
| Late binding as a liveness mechanism | — | — | — |
| Namespacing of member names | — | — | — |
| Bindings introduced by patterns | — | — | — |
| Redefinition in a live session | — | — | — |

This section has more implementation than spec. Scopes, shadowing, capture analysis, and
the prelude all work — they were decided in §statements, §functions, and `core-ir.md` and
built in `ir::lower`. The two top rows that changed on 2026-09-07 are this section's own
distinction, decided in §statements because that is where the declaration form lives; see
*Glue Semantics* below. The rows that stay empty are the ones that need modules or an
image to mean anything.

---

## Checklist

- **Lexical scoping** **[Lox]**, ch. 11 — resolving and binding
  - Block scope **[Lox]** vs. function scope vs. hoisting
  - The "use before declaration in same scope" rule (Lox makes it a static error)
  - Closure capture resolution as a separate static pass (§functions)
- **Globals** **[Lox]** — late-bound in Lox (ch. 21) vs. locals (ch. 22) which are
  slot-resolved. The distinction matters for wasm: locals → wasm locals, globals →
  wasm globals or memory
- **Late binding as a liveness mechanism** — which references are resolved at compile
  time and which stay indirect, and what that costs (goal §liveness)
- **Shadowing** — within a scope, across nested scopes, and across imports (§modules)
- **Name resolution order** — locals, enclosing scopes, module scope, imports, prelude
- **Namespacing of member names** — do fields and methods share a namespace (Lox: yes,
  §objects)? Do types and values? Do operations on an instance?
- **Bindings in patterns** — what a pattern match introduces, and its scope (§unions)
- **Redefinition in a live session** — rebinding a name at a prompt or in a running image,
  including to a different type (§inference, goal §image)

## Glue Syntax

## Glue Semantics

> Decided elsewhere. This section still owes its own rules; what is settled lives where it
> was decided, and is indexed here so there is one place to look.

- **Block scope, shadowing, and use-before-declaration** — §statements.
- **Capture as a separate static pass**, and what it does to a binding — §functions and
  [`../core-ir.md`](../core-ir.md).
- **Locals versus globals. Decided 2026-09-07, in §statements.** A top-level binding is a
  **global**, and every function in the file may read one whichever order the two are
  written in; a binding inside a block is a **local**, and a nested `fn` still cannot see
  one. That is this section's own distinction — Lox's ch. 21 globals against ch. 22 locals
  — arriving from §statements because that is where the declaration form lives, and it
  lands the way this section predicted it would matter: a local becomes a wasm local, a
  global becomes a wasm global.

  **Late binding is untouched by it.** A global is resolved statically to an index, not
  looked up by name at each use, so none of goal §liveness' redefinition story is bought
  or spent here. That question is still open and still this section's.
