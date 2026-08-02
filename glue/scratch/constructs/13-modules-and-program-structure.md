# §13 — Modules & Program Structure

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox omits modules entirely — one file, one global scope. Glue can't, and this is the
section where the novelty budget (the meta-rule in the index) is most likely to be spent on purpose
rather than by accident, because Othismo already has a structure that a conventional
module system would duplicate or fight.

**The collision.** A module is normally a compile-time thing: a file, a namespace for
names, resolved by the compiler, gone by runtime. Othismo's namespace is a *runtime*
thing: live instances addressed by path, exchanging BSON messages of the form
`/path.operation`, persisted in an image. Goal §2.4 wants these to line up — if the units
of program structure are the units of runtime structure, then a running program is already
an addressable, inspectable graph and telemetry falls out of the design. Goal §4.2 warns
what happens if they line up *too* well: every module boundary becoming a message boundary
would be beautiful and far too slow.

So the real question here isn't import syntax. It's: **what is the relationship between a
name you import and a path you can send a message to?** Some available positions, none
free:

- **Independent.** Modules are compile-time only; instances are a runtime concept the
  language addresses through an API. Conventional, cheap, and gives up goal §2.4's thesis.
- **Isomorphic.** A module *is* an instance; importing is establishing a reference to a
  live object. Maximally Smalltalk, maximally introspectable, and pays §4.2's cost on
  every call.
- **Two-tier.** Modules are compile-time; some declarations (§11's `actor`/`instance`)
  additionally publish into the namespace. Probably the pragmatic answer, and the cost is
  that the language now has two kinds of boundary and must make the difference legible.

**The liveness angle** (goal §4.1). Static linking makes replacement impossible; late
binding is what lets a module be swapped in a running image. That's a module-system
decision as much as a runtime one, and it interacts with monomorphization (§8), inferred
interfaces (§10), and whether a module's exports are a stable ABI (§16).

**The image angle** (goal §4.5). If code can be defined into a live image, then "import"
may resolve against the image rather than the filesystem — and the question of whether
source text or image state is authoritative stops being philosophical and becomes a
resolution algorithm.

## Checklist

- **Unit of modularity** — file, directory, or an explicit `module` declaration; nesting;
  one file = one module?
- **Import forms** — whole-module, selective, aliasing, wildcard/glob, re-export;
  qualified vs. unqualified use; whether imports are declarations or expressions
  (an interactive prompt wants the latter — goal §2.1)
- **Export & visibility** — explicit export lists vs. per-declaration modifiers;
  public / private / module-private; visibility across nesting (§12)
- **Resolution** — relative paths, a registry, or Othismo namespace paths; search order;
  ambiguity and shadowing between modules (§12); versions
- **Cycles** — permitted, rejected, or resolved lazily
- **Compilation unit & separate compilation** — interface files, inferred interfaces
  (§10), what must be re-checked when a dependency changes
- **Initialization** — module-level side effects, initialization order, and what "loading"
  means in an image that's already running (§15)
- **Entry point** — top-level statements, `main`, or exported handlers **[wasm]**. wasm
  offers `_start`, exports, and a `start` section; Othismo calls `_message_received`.
  Goal §2.1 forbids a *required* `main`, which makes top-level statements the default and
  the exported handler the thing you add later
- **The Othismo mapping** — the load-bearing decisions
  - Does a Glue module compile to a wasm module, an image object, both, or neither?
  - Does importing mean linking, or acquiring a reference to a live instance?
  - Is there syntax for "this declaration is addressable in the namespace" (§11)?
  - Does a module path and a namespace path use the same syntax? Should they?
  - How does a module declare what it needs from the host (§5's foreign functions)?
- **Late binding & replacement** (goal §4.1) — can a module be replaced in a running
  image; what happens to instances holding references to the old one; state migration;
  whether the language has syntax for versioned or migratable state
- **Capability scoping** — Plan 9's per-process namespace as prior art: is what a module
  can *reach* part of its declaration, or ambient?
- **Standard library** — modules, host imports, or a prelude; and what's in scope with no
  imports at all (goal §2.1's zero-ceremony one-liner)
- **Source of truth** (goal §4.5) — does an import resolve against files or the image; how
  a definition made live gets captured back into source; whether an image is rebuildable
  from source

## Glue Syntax

## Glue Semantics
