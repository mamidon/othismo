# Language Construct Checklist

A completeness checklist for designing `glue`, a language compiled to WebAssembly.

Structure follows *Crafting Interpreters* (craftinginterpreters.com) — Lox's grammar is
the spine, because Lox is deliberately the *minimum* set of constructs that still feels
like a real language. Anything Lox has, you almost certainly need. Anything Lox omits is
listed separately, because Lox's omissions are choices, not oversights, and several of
them (integers, modulo, bitwise ops) stop being optional the moment you target wasm.

This file is the index. Each section lives in its own file under `constructs/`, holding
a summary, the checklist of decisions in that area, and the Glue syntax and semantics
once they're decided.

Legend used throughout the section files:

- **[Lox]** — present in Lox; treat as baseline
- **[Lox-omits]** — Bob Nystrom explicitly leaves this out
- **[wasm]** — the wasm target forces a decision here that a tree-walking interpreter could defer

**Decided is not the same as implemented**, so the table tracks both. *Syntax* and
*Semantics* record what has been **decided**; *Implementation* records what is **built** in
`glue/`. The two move independently, and the table is worth reading for where they
disagree: §scope has working scoping and no written spec, §comptime is specified in detail
and implemented nowhere.

Several decided constructs are deliberately absent from `glue/`, so that the implemented
language stays small enough to think about while the parts that make Glue *Glue* are still
being found. Their spec text stays where it is, marked **✂**, and
[Deferred decisions](constructs/deferred.md#cut-from-the-core) lists them with what each
costs and what comes back with it.

Each section file opens with the same table for its own sub-areas, so this one is a
summary of sixteen others rather than the only place progress is recorded.

---

## Sections

Status: **✓** done · **◑** partial · **—** none · **·** not applicable · **✂** cut from the core

| § | Section | Covers | Syntax | Semantics | Impl |
| --- | --- | --- | --- | --- | --- |
| §lexical | [Lexical structure](constructs/01-lexical-structure.md) | Identifiers, literals, comments, layout | ◑ | ◑ | ✓ |
| §expressions | [Expressions](constructs/02-expressions.md) | Operators, precedence, expression forms | ◑ | ◑ | ◑ |
| §statements | [Statements and declarations](constructs/03-statements-and-declarations.md) | Statement/expression split, blocks, bindings | ◑ | ◑ | ◑ |
| §control | [Control flow](constructs/04-control-flow.md) | Branching, looping, iteration, async | ◑ | ◑ | ✓ |
| §functions | [Functions](constructs/05-functions.md) | Declarations, closures, parameters, host functions | ◑ | ◑ | ✓ |
| §types | [Data and types](constructs/06-data-and-types.md) | Primitives, aggregates, memory, representation | ◑ | ◑ | ◑ |
| §unions | [Unions and pattern matching](constructs/07-unions-and-pattern-matching.md) | Sum types, patterns, exhaustiveness | — | — | — |
| §generics | [Generics and polymorphism](constructs/08-generics.md) | Type parameters, bounds, monomorphize vs. box | ◑ | ◑ | — |
| §errors | [Error handling](constructs/09-error-handling.md) | Exceptions vs. results, traps, failure across messages | — | ◑ | ◑ |
| §inference | [Type inference and gradual typing](constructs/10-type-inference.md) | Optional annotations, inference scope, dynamic values | ◑ | ◑ | ◑ |
| §objects | [Objects and abstraction](constructs/11-objects-and-abstraction.md) | Classes/actors, dispatch, inheritance | ◑ | — | — |
| §scope | [Scope and name resolution](constructs/12-scope-and-names.md) | Lexical scoping, shadowing, late binding | ◑ | ◑ | ◑ |
| §modules | [Modules and program structure](constructs/13-modules-and-program-structure.md) | Imports, visibility, entry point, Othismo namespace | ◑ | ◑ | ◑ |
| §comptime | [Metaprogramming and tooling](constructs/14-metaprogramming-and-tooling.md) | Macros, attributes, debug info, tests | ◑ | ◑ | ◑ |
| §semantics | [Semantics without syntax](constructs/15-invisible-semantics.md) | Evaluation order, coercion, traps, reentrancy | · | ◑ | ◑ |
| §wasm | [wasm target decisions](constructs/16-wasm-target-decisions.md) | Cross-cutting constraints from the target | · | ◑ | — |

Alongside the numbered sections, [**Deferred decisions**](constructs/deferred.md) is a
companion register of things consciously postponed — what was deferred, why, and what it
blocks. Sections link to it rather than carrying the argument themselves. It holds two
kinds: **undecided**, where the answer isn't written anywhere, and **cut from the core**,
where the answer is written in its section and the implementation deliberately lacks it.
It also indexes the **Open** questions that remain owned by their own sections, so there's
one place to look for everything unresolved.

[**Core IR**](core-ir.md) is a third companion document, and the only one that is not
about the surface language: it specifies the typed, monomorphic representation §comptime's
elaboration lowers to, and which the interpreter and the wasm back end both consume. It
sits outside the numbering because it is not a construct.

§semantics is expected to stay empty under *Syntax* by construction — if something lands
there, it probably belongs in another section. §wasm is a constraint set rather than a
feature area, and may end up recording where each decision went rather than a spelling of
its own.

Five sections cover things Lox lacks and Glue wants, and are the reason the numbering
past §control differs from the book's: **§unions**, **§generics**, **§errors**,
**§inference**, and **§modules**.

§unions–§inference are ordered by dependency rather than by importance. Unions come first
because `Result` and `Option` are unions; generics next because those unions are
parameterized; error handling then has almost nothing left to invent beyond a propagation
operator; and inference last, since it has to account for everything the four of them
introduced.

§modules is the largest deliberate novelty spend, because Othismo's runtime namespace and
a conventional compile-time module system are two answers to the same question.

---

## The one meta-rule

Design Note: Novelty Budget (ch. 28) — every unfamiliar construct spends from a fixed
budget of the user's willingness to learn. Use this list to be *deliberate* about
omissions, not to maximize inclusion. Lox is small on purpose, and it's still a language.

This has no section file because it has no syntax or semantics of its own; it applies to
every choice recorded in the sixteen that do.

## Sources

- craftinginterpreters.com — full book, especially:
  - Ch. 3 *The Lox Language* — the feature set
  - Appendix I *Lox Grammar* — the grammar quoted in the section files
  - Ch. 5–13 — tree-walk semantics; ch. 11 for scope/binding
  - Ch. 17–29 — compilation concerns that mirror wasm codegen
  - The Design Notes, which are where the decision points live

## Related

- `design-goals.md` — the goals these decisions must be traceable to
