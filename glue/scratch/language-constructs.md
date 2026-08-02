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

---

## Sections

Status: **—** not started · **◑** partially defined · **✓** defined

| § | Section | Covers | Syntax | Semantics |
| --- | --- | --- | --- | --- |
| 1 | [Lexical structure](constructs/01-lexical-structure.md) | Identifiers, literals, comments, layout | ◑ | ◑ |
| 2 | [Expressions](constructs/02-expressions.md) | Operators, precedence, expression forms | ◑ | ◑ |
| 3 | [Statements and declarations](constructs/03-statements-and-declarations.md) | Statement/expression split, blocks, bindings | ◑ | ◑ |
| 4 | [Control flow](constructs/04-control-flow.md) | Branching, looping, iteration, async | — | — |
| 5 | [Functions](constructs/05-functions.md) | Declarations, closures, parameters, host functions | — | — |
| 6 | [Data and types](constructs/06-data-and-types.md) | Primitives, aggregates, memory, representation | — | — |
| 7 | [Unions and pattern matching](constructs/07-unions-and-pattern-matching.md) | Sum types, patterns, exhaustiveness | — | — |
| 8 | [Generics and polymorphism](constructs/08-generics.md) | Type parameters, bounds, monomorphize vs. box | — | — |
| 9 | [Error handling](constructs/09-error-handling.md) | Exceptions vs. results, traps, failure across messages | — | — |
| 10 | [Type inference and gradual typing](constructs/10-type-inference.md) | Optional annotations, inference scope, dynamic values | — | — |
| 11 | [Objects and abstraction](constructs/11-objects-and-abstraction.md) | Classes/actors, dispatch, inheritance | — | — |
| 12 | [Scope and name resolution](constructs/12-scope-and-names.md) | Lexical scoping, shadowing, late binding | — | — |
| 13 | [Modules and program structure](constructs/13-modules-and-program-structure.md) | Imports, visibility, entry point, Othismo namespace | — | — |
| 14 | [Metaprogramming and tooling](constructs/14-metaprogramming-and-tooling.md) | Macros, attributes, debug info, tests | — | — |
| 15 | [Semantics without syntax](constructs/15-invisible-semantics.md) | Evaluation order, coercion, traps, reentrancy | — | — |
| 16 | [wasm target decisions](constructs/16-wasm-target-decisions.md) | Cross-cutting constraints from the target | — | — |

Alongside the numbered sections, [**Deferred decisions**](constructs/deferred.md) is a
companion register of things consciously postponed — what was deferred, why, and what it
blocks. Sections link to it rather than carrying the argument themselves. It also indexes
the **Open** questions that remain owned by their own sections, so there's one place to
look for everything unresolved.

§15 is expected to stay empty under *Syntax* by construction — if something lands there,
it probably belongs in another section. §16 is a constraint set rather than a feature
area, and may end up recording where each decision went rather than a spelling of its own.

Five sections cover things Lox lacks and Glue wants, and are the reason the numbering
past §4 differs from the book's: **§7 unions and matching**, **§8 generics**,
**§9 error handling**, **§10 inference**, and **§13 modules**.

§7–§10 are ordered by dependency rather than by importance. Unions come first because
`Result` and `Option` are unions; generics next because those unions are parameterized;
error handling then has almost nothing left to invent beyond a propagation operator; and
inference last, since it has to account for everything the four of them introduced.

§13 is the largest deliberate novelty spend, because Othismo's runtime namespace and a
conventional compile-time module system are two answers to the same question.

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
