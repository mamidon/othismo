# §4 — Control Flow

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox's control flow is `if`/`else`, `while`, a C-style three-clause `for` (desugared to
`while`), and `return`. That is enough to be Turing-complete and not enough to be
pleasant. The omissions that hurt most are `for-in` with an iteration protocol,
`break`/`continue`, and error handling of any kind.

Two forms of control flow have their own sections because they carry a type-system
component this one doesn't: **error handling** (§9) and **pattern matching** (§7).

Two of the constructs on that list — iteration and async — need things Glue doesn't have
yet, and are left out rather than sketched. What remains is the branching and looping a
language needs to work at all.

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

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Not decided here

- **Labelled `break` / `continue`** — deferred. See [Deferred decisions](deferred.md).
- **`match` arms and patterns** — §7's. §4 records only that `match` is an expression (§2)
  and that it is how branching on shape is spelled.
- **Error handling** — §9's entirely: exceptions versus results, traps, propagation, and
  cleanup.
- **Iteration and async** — both need constructs that don't exist yet, so neither is
  designed here. Iterating requires an iterable (§6) and, for the counting case, ranges;
  awaiting requires something to await, which is a reply to a message (§11, §13). They
  come back when their prerequisites do.

### Conditionals

```
if x > 0 {
  …
} else if x < 0 {
  …
} else {
  …
}
```

- **Braces are mandatory; the condition is unparenthesized.** Together these delete the
  dangling-else problem outright — there is no way to write an `else` whose owner is
  ambiguous, so no disambiguation rule is needed.
- `else if` is `else` followed by another `if`, not a distinct `elif` keyword. It falls
  out of `else` accepting either a block or an `if`.
- A map literal in the condition needs parentheses (§1) — the one place users trip.
- `if` is an expression (§2). With no `else` its type is unit, so it can be a statement
  but not a value.

### Loops

```
while ready() {
  …
}
```

- **`while` is the only loop.** It takes a `bool` condition; there is no truthiness (§2).
- The body is a block and introduces a scope (§12).
- **Loops are statements, not expressions.** Their value is unit. This is why there is no
  `loop` keyword and no `break` with a value: those exist to make an infinite loop produce
  something, and `while true { … }` covers the control flow without the extra construct.

There is no `for` of any kind. The C-style three-clause form is `while` with extra syntax,
and the `for … in` form needs something to iterate — an iterable (§6) and, for counting,
ranges. Neither is worth adding a keyword for until then.

### `break`, `continue`, `return`

```
break;
continue;
return;
return value;
```

- `break` and `continue` apply to the innermost enclosing loop. Unlabelled only, for now.
- `return` exits the enclosing function. It is for *early* exit: a function body is a
  block, so its ordinary result is the trailing expression (§2, §3), and a well-shaped
  function often has no `return` at all.
- `return` inside a block expression still returns from the function, not the block (§2).

### Declined

- **`goto`** — no. wasm has none either: it offers structured `block` / `loop` / `br` /
  `br_if` / `br_table`, so a source-level `goto` would have to be *compiled into*
  structured form. The target is telling us something and we're listening.
- **`do-while` / `repeat-until`** — a loop whose condition is at the bottom is rare enough
  that `while true { …; if !c { break; } }` is an acceptable price, and it's one fewer
  keyword and one fewer precedence question.
- **`loop`** — `while true` is already boring and conventional.
- **`switch` with fallthrough** — `match` (§7) is the better version of the same idea, and
  fallthrough is a bug generator with a compatibility argument Glue doesn't have to honor.
- **`while … else`** — Python's loop-else confuses nearly everyone who meets it.

---

## Glue Semantics

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Conditions

Every condition — `if`, `while` — must have type `bool`. Not "must be convertible to
bool": there are no implicit conversions (§2) and no truthiness, so the only thing that
can appear there is a `bool`.

### Loop values and termination

- A loop's value is unit. A `while` whose body never runs is still unit, so there is no
  "what if it never executes" question to answer.
- **Non-termination is not an error.** An infinite loop in a guest instance blocks that
  instance; whether the host can interrupt it is a runtime concern (§16's resource limits,
  §15's observability), not a language one. Worth stating because a language with a
  host-driven message loop invites the assumption that the host can always regain control,
  and it cannot, absent a mechanism nobody has designed yet.

### Structured control flow

Every construct here is structured, and deliberately so: control enters a block at the
top and leaves through `break`, `continue`, `return`, or the end. That is what wasm's
`block` / `loop` / `br` family expresses directly, so the compiler's job stays a
translation rather than a reconstruction — and it is what keeps the door open for
suspension points inside loops later, since a structured region is something a transform
can split.
