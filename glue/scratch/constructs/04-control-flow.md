# §4 — Control Flow

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox's control flow is `if`/`else`, `while`, a C-style three-clause `for` (desugared to
`while`), and `return`. That is enough to be Turing-complete and not enough to be
pleasant. The omissions that hurt most are `for-in` with an iteration protocol,
`break`/`continue`, and error handling of any kind.

Two forms of control flow have their own sections because they carry a type-system
component this one doesn't: **error handling** (§9) and **pattern matching** (§7).

Async is a live question rather than a nicety: Othismo's guest is single-threaded with a
host-driven message loop, and the language's concurrency model has to agree with the
runtime's. Othismo's messages are re-entrant and interleaved, which control flow has to
account for (see §15).

## Checklist

- **`if` / `else`** — `"if" "(" expression ")" statement ( "else" statement )?` **[Lox]**
  - Dangling-else resolution (bind to nearest `if`)
  - Parens required or not; braces required or not
  - `else if` chains vs. a distinct `elif`
- **`while`** — **[Lox]**
- **`for`** — C-style three-clause **[Lox]**, desugared to `while` in the book
  (see Design Note: Spoonfuls of Syntactic Sugar, ch. 9)
- **`for-in` / iteration protocol** **[Lox-omits]** — probably the single most missed
  construct; requires deciding on an iterator/iterable interface
- **`do-while`** / `repeat-until` **[Lox-omits]**
- **Infinite loop** (`loop`)
- **`break` / `continue`** **[Lox-omits]** — and labeled variants for nested loops
- **`return`** — `"return" expression? ";"` **[Lox]**
  - Implicit return of last expression
  - Multiple return values
  - Returning from top level; returning from an initializer (Lox: banned)
- **`match` / pattern matching** **[Lox-omits]** — see §7, including `switch`-with-fallthrough
  as the worse version of it, and `if let` / `while let` as its loop and branch forms
- **`goto`** — no (see Design Note: Considering Goto Harmful, ch. 23) **[wasm]** — worth
  noting wasm *has* no goto; it has structured `block`/`loop`/`br`/`br_if`/`br_table`,
  which is a good reason not to want one
- **Error handling** — Lox has none at all; see §9 for the whole family (exceptions,
  Result types and propagation, traps vs. recoverable errors, `defer`/cleanup)
- **Async control flow** — relevant to othismo: `async`/`await`, or CPS, or one-shot
  continuations. Note wasm's single-threaded guest + host-driven message loop means the
  language's concurrency model and the runtime's must agree

## Glue Syntax

## Glue Semantics
