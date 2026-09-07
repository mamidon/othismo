# §errors — Error Handling

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox has no error handling at all — a runtime error prints and exits. Every real language
needs an answer, and the answer is structural: it shows up in function signatures, in the
type system, in control flow, and at the host boundary.

This section sits after unions (§unions) and generics (§generics) deliberately, because
the leading candidate is built out of both: `Result[T, E]` and `Option[T]` are a
discriminated union with a type parameter. Decide those first and error handling is mostly
a matter of choosing a propagation operator; decide error handling first and you'll end up
specifying the same machinery twice, worse.

The two families are exceptions (`throw`/`try`/`catch`/`finally`) and errors as values
(`Result`/`Option` plus a propagation operator). Three things push Glue toward the second:

- **wasm.** Exception handling is proposal-level, and unwinding across the host boundary
  is genuinely hard. Result types need nothing the target doesn't already have.
- **Othismo.** An error crossing an instance boundary is a *message*, not a stack unwind.
  A failed operation replies with something, and that something has a BSON encoding.
  Whatever the in-language error type is, it has to survive that trip.
- **It's already paid for.** If §unions and §generics land, `Result` and `Option` are
  ordinary library types rather than language features, and propagation is the only new
  syntax.

There's a second axis that matters as much as the first: which failures are *recoverable*
and which are traps. wasm traps on integer division by zero, out-of-bounds access, and
stack exhaustion whether or not the language wants it to, so the trap category exists
already; the question is what else joins it (integer overflow? a failed allocation?) and
whether a trapping instance is dead or restartable.

Goal §scale is the live tension. Silent failure and terse errors make one-liners pleasant
and ten-thousand-line programs untrustworthy; this is one of the features where "choose
per-feature, repeatedly" has to produce an actual rule.

## Status

Legend in the [index](../language-constructs.md). *Syntax* and *Semantics* track what has
been **decided**; *Implementation* tracks what is **built** in `glue/`.

| Area | Syntax | Semantics | Implementation |
| --- | --- | --- | --- |
| Traps — overflow, division by zero, stack exhaustion | · | ✓ | ✓ |
| Error taxonomy — compile error, trap, recoverable failure | — | ◑ | ◑ |
| Whether a trap is recoverable | — | — | — |
| `Result` and `Option` as library types | — | — | — |
| Propagation operator (`?`) | — | — | — |
| Error type unification and added context | — | — | — |
| Cleanup — `defer`, scope guards | — | — | — |
| Failure at the message boundary | — | — | — |
| Supervision and recovery | — | — | — |
| Interactive behaviour at a prompt | — | — | — |
| Diagnostics — provenance, traces, telemetry | — | — | ◑ |

The trap row is decided in §expressions and §functions and implemented; the taxonomy
around it is not written down here yet, which is why the second row is ◑ rather than ✓.
Diagnostics are ◑ because every IR node carries provenance back to a CST node, which is
the machinery a trace would use, with nothing yet reading it.

---

## Checklist

- **Error taxonomy** — what is a compile error, a recoverable runtime error, a trap, a
  panic. Ideally no undefined behavior at all (see also §semantics)
  - Which arithmetic failures are which **[wasm]** — overflow, division by zero, invalid
    float conversion
  - Allocation failure and memory-growth failure as observable events **[wasm]**
- **Exceptions** — `throw` / `try` / `catch` / `finally` **[wasm]**
  - Checked vs. unchecked; typed catch clauses; catch-by-pattern (§unions)
  - Whether they can cross the host boundary at all
  - Interaction with the interpreter tier (goal §both-modes — both back ends must agree)
- **Errors as values**
  - `Result` / `Option` as generic (§generics) library types over sum types (§unions), or
    as built-ins
  - Propagation operator (`?`) — and what it does to a function's signature
  - Error type unification: one error type, a hierarchy, or an open union
  - Adding context while propagating (the thing that makes error values usable)
- **Ergonomics vs. discipline** — must errors be handled, or can they be dropped? A
  warning, a static error, or an explicit `ignore`
- **Cleanup** — `defer`, RAII/scope guards, `finally`, `using`. Which one survives both
  execution modes and the wasm memory model (§types)
- **Failure at the message boundary** — Othismo-specific, and probably the load-bearing
  decision here
  - Does a failed `/path.operation` reply with an error document, or fail to reply?
  - Error representation in BSON; whether error *types* are shared across instances
  - Timeouts and unreachable instances as a distinct error class the language must name
  - Re-entrancy: an error raised while a handler is suspended awaiting a reply
    (§semantics)
- **Supervision and recovery** — Erlang's let-it-crash as an alternative to in-language
  recovery: is a trapping instance restarted, and by whom? Does the language have syntax
  for supervision, or is it purely a runtime concern?
- **Interactive behavior** — an error at a prompt must not take down the session, and goal
  §one-language says the prompt and the program are the same language. What differs is
  therefore the *handler*, not the semantics
- **Diagnostics** — stack traces (needs §comptime's debug info), error provenance, and
  whether errors are automatically visible to telemetry (goal §living)

## Glue Syntax

## Glue Semantics
