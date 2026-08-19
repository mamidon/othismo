# Deferred Decisions

> Companion register to the numbered sections. Index: [`../language-constructs.md`](../language-constructs.md)

Things consciously **postponed**, not rejected and not overlooked. Each entry records why
it was deferred, what it would cost to decide wrongly now, and what it blocks — so the
argument doesn't have to be reconstructed when it comes back up.

Two kinds of thing live here, and they are not the same kind.

**Undecided** — the question is open and the answer isn't written anywhere. These are the
original contents of this register.

**Cut from the core** — the question *is* decided, the answer is written in its section,
and the implementation deliberately doesn't have it. Not a retreat: a smaller language is
easier to think about while the parts that make Glue *Glue* are still being found, and
every one of these is additive. The spec text stays where it is, marked, so adding one
back is a matter of implementing what's already written rather than deciding it again.

Questions actively owned by a later section stay in that section, marked **Open**; the
last part of this file points at them so there's one place to look.

## Undecided

| Deferred | Raised in | Lands in | Blocks |
| --- | --- | --- | --- |
| [Range syntax](#range-syntax) | §2 | §2 | slicing (§6), iteration (§4) |
| [Pipeline operator](#pipeline-operator) | §2 | §2 | nothing; waits on §11 |
| [String interpolation](#string-interpolation) | §1 | §1 | nothing; wants §11 |
| [Multi-line indentation stripping](#multi-line-indentation-stripping) | §1 | §1 | waits on `"""` returning |
| [Labelled break and continue](#labelled-break-and-continue) | §4 | §4 | nothing |
| [Comptime access to the image](#comptime-access-to-the-image) | §14 | §14 | self-hosted compilation |

## Cut from the core

**Cut 2026-08-09.** Each is specified in the section named, and implemented nowhere.

| Cut | Specified in | Costs today |
| --- | --- | --- |
| [Doc comments](#doc-comments) | §1, §14 | no attached documentation; `///` is an ordinary comment |
| [Raw and multi-line strings](#raw-and-multi-line-strings) | §1 | escapes are the only way to write a quote or a newline |
| [Bitwise and shift operators](#bitwise-and-shift-operators) | §2 | no `& \| ^ ~ << >>`; the ladder loses four rungs |
| [Compound assignment](#compound-assignment) | §3 | `a = a + b` is the only spelling |
| [Placeholder punctuation](#placeholder-punctuation) | §1 | `::`, `..`, `...` lex as their parts |

---

## Range syntax

**Deferred 2026-08-02.** Raised in §2.

Interval notation (`a..b`, `[a..b)`, `(a..b)`, `(a..b]`, `[a..b]`) reads best in isolation
and was the leading candidate. It collides with three existing roles for the same
delimiters:

- **Grouping parens** — `(0..n).map(f)` would change meaning rather than merely group.
- **List literals** — `[0..2]` is either an inclusive range or a one-element list holding
  a range.
- **The index operator** — in `arr[0..2]`, the outer bracket is either the subscript or a
  bound marker.

The first two have workable disambiguation rules (the bracket forms double as the grouped
forms; a trailing comma marks a list). The third needs an arbitrary one, and slicing is
where ranges get used most. Separately, mismatched pairs like `[0..2)` are unbalanced to
every tool that counts brackets — editors, formatters, bracket matching, `%` in vim.

**Re-examine this when collections arrive.** §6 has no lists and no indexing beyond `Str`,
so two of those three collisions are currently hypothetical. They return with §8's generic
collections — which is also when ranges become urgent, so the analysis and the need arrive
together rather than one ahead of the other.

The collision-free alternative puts the markers on the operator, at a real cost in
readability:

```
a..b      →  [a, b)
a..=b     →  [a, b]
a<..b     →  (a, b)
a<..=b    →  (a, b]
```

**Nothing is foreclosed.** §1's ban on a trailing `.` in float literals keeps `..`
available as an operator whenever this is decided. It no longer lexes as one token —
see [Placeholder punctuation](#placeholder-punctuation) — but that is one line of the
scanner, not a constraint. Slicing (§6) and iteration (§4) both depend on the outcome, so
this is the deferral most likely to come due first.

## Pipeline operator

**Deferred 2026-08-02.** Raised in §2.

The shell is explicitly the thing to beat on immediacy (goal §2.5), and a typed `|>` is
that pipeline. It also lets free functions join a chain without being declared as methods
on a type.

It waits on §11. Until `x.f()` is settled as a method call or a message send, adding `|>`
risks ending up with three call syntaxes for the same idea. Cheap to add later, awkward to
remove — which is the asymmetry that makes deferring correct rather than merely cautious.

## String interpolation

**Deferred 2026-08-02.** Raised in §1.

`"a${x}b"` is a *parser* feature rather than a lexer one, and it needs a conversion
interface (§11) to define what each interpolated part becomes. Neither exists yet.

Deferring is cheap: §1's lexer is a plain scanner with no mode stack, and interpolation is
the only construct that would nest a full expression inside a token. Adding it later is
additive — nothing in the current string grammar forecloses it. Adding it *now* would mean
inventing the conversion interface in passing, in the wrong section.

## Multi-line indentation stripping

**Deferred 2026-08-02.** Raised in §1.

A `"""` string takes its content literally, including every leading space on every line.
Whether it should strip indentation to match its closing delimiter is undecided.

**Doubly deferred:** `"""` itself is [cut from the core](#raw-and-multi-line-strings), so
this question has no subject at the moment. It comes back when the literal does, and the
two should be decided together — bringing back the literal without settling this is what
produces the version a language later regrets.

The feature is wanted; the exact rule is fiddly, and several languages have shipped a
version they later regretted. Since goal §3 puts Glue under no backward-compatibility
obligation during design, changing the rule later is allowed — but it *is* a change in
meaning for existing strings, which is the one reason to prefer getting it right over
getting it early.

## Labelled break and continue

**Deferred 2026-08-02.** Raised in §4.

`break` and `continue` currently apply to the innermost loop only. Escaping a nested loop
therefore needs a flag variable or extraction into a function that returns early — the
former is the classic bug source, the latter is usually the better design anyway.

Deferred rather than declined because labels are cheap and additive, and because the
"extract a function" answer stops being free once closures capture (§5). What's missing is
a spelling: §1 admits no sigils in identifiers, so Rust's `'outer` is unavailable, and a
label form (`outer: while …`) would need to not collide with whatever §7 does with `:` in
patterns — or with map literals, if collections bring them back.

## Comptime access to the image

**Deferred 2026-08-18.** Raised in §14.

§14 makes comptime **hermetic**: compile-time evaluation sees pure computation over
comptime-known values and the declarations of the compilation unit, and nothing else — no
image, no namespace, no messages, no host imports, no clock, no randomness.

Othismo makes the opposite genuinely interesting. Every other language's compiler runs
outside the world its output will live in; Glue's could run *inside* one. Comptime code
could ask the namespace which instances exist, read an instance's shape, or specialize
against what is actually deployed. That is not a feature other languages could copy, which
is the usual sign that it is worth the novelty budget (goal §2.3).

It is deferred rather than declined for three reasons, in order of how much they cost:

- **Reproducibility.** A program whose meaning depends on image state cannot be rebuilt
  from source — goal §4.5's hazard at its sharpest, met in the worst possible place.
- **Tooling.** Deterministic comptime is what lets the language server cache
  instantiations across keystrokes. A comptime that reads mutable state invalidates every
  cached instantiation on every edit, and §14's fuel budget stops being an upper bound on
  anything.
- **It needs a compiler that runs in the image at all.** The compiler is a native Rust
  crate today. Compiling *it* to wasm and hosting it as an Othismo instance is the
  prerequisite, and it is not close.

**The asymmetry is the argument.** Opening this later is additive: programs written
against a hermetic comptime keep working when it stops being hermetic. Closing it later
breaks every program that reached out. Start closed.

**What it would take to open.** A defined comptime world — which queries exist, what they
return, and what happens when the image changes between two compilations of the same
source. That is a §13 question as much as a §14 one, since it is the namespace being
addressed. Nothing in §14 forecloses it: `comptime` already denotes a stage, and giving
that stage more capabilities does not change what the keyword means.

---

## Doc comments

**Cut 2026-08-09.** Specified in §1, used by §14.

`///` was a distinct token — not trivia — attaching to the declaration that followed it,
with a warning when it attached to nothing. That warning was the one diagnostic §1
assigned to the parser rather than the lexer.

It is now an ordinary line comment. Nothing reads documentation yet: §14 is unstarted, so
there is no formatter, no hover, and no doc generator to consume what a `///` would carry.
A token that only one unwritten section wants is a token that can wait.

**What comes back with it.** The `DocComment` kind and its non-trivia status; the rule
that exactly three slashes make one and a fourth takes it back; the parser counting them
ahead of a declaration so they land inside it; and the dangling-doc-comment warning, which
is currently the only thing that would have used `Severity::Warning`.

## Raw and multi-line strings

**Cut 2026-08-09.** Specified in §1.

`r"…"` took no escapes and could not contain a quote at all. `"""…"""` was the only
literal that spanned lines.

One string form is enough to write every program the core can express, and the two extra
forms carried more than their weight in machinery: a second and third scanner arm, two
diagnostic kinds, two decode paths, and the `r`-glued-to-a-quote special case in the
identifier arm.

**A consequence worth recording.** §1 asks for CRLF to be normalized to LF before lexing.
With `"""` gone, no literal can contain a raw newline — every string stops at one — so
there is nowhere for normalization to happen and the code for it is gone. It returns when
a multi-line form does, and the two are the same decision.

[Indentation stripping](#multi-line-indentation-stripping) waits on this.

## Bitwise and shift operators

**Cut 2026-08-09.** Specified in §2.

`&` `|` `^` `~` on integers, and `<<` `>>` with §2's arithmetic-versus-logical split
decided by signedness.

wasm has all of these as instructions, so they cost almost nothing to implement and were
never the hard part. They are cut because they are *uninteresting*: no question about
Glue's identity turns on them, and they take four rungs of the precedence ladder with
them — including §2's correction to C, where bitwise binds tighter than comparison.

**What comes back with it.** Four levels between `+` and comparison, that correction and
the argument for it, unary `~`, and `>>` needing no `>>>` companion because the `s`/`u`
split from §1 already decides its behaviour.

Related, and separately owned: the rotate, popcount, clz, and ctz intrinsics §2 hands to
§6 as library functions rather than operators. Those were never syntax.

## Compound assignment

**Cut 2026-08-09.** Specified in §3.

`+=` `-=` `*=` `/=` `%=` `&=` `|=` `^=` `<<=` `>>=`. Each is exactly `a = a op b` with the
place evaluated once, and each traps on overflow like the operator it wraps.

Ten tokens for a rewrite the reader can do, and half of them are spelled from operators
that are themselves cut. "The place is evaluated once" is the only part with semantic
content, and it has no observable consequence until a place can have side effects —
which needs §11.

**What comes back with it.** Ten tokens, one line of the parser's assignment rule, and the
evaluate-the-place-once guarantee, which should be written down when there is something
that can observe it.

## Placeholder punctuation

**Cut 2026-08-09.** Specified in §1.

`::`, `..`, and `...` lexed as single tokens despite no construct using them. §1 justified
this explicitly as a deliberate exception, on the grounds that a `..` token lets the parser
say "ranges aren't implemented" where three separate tokens produce a message about
numbers instead.

The exception was worth stating and is not worth keeping. It optimizes an error message
for a construct nobody can write, and it costs a reader of the token list the ability to
tell what the language actually has. `0..5` now lexes as `INT DOT FLOAT` exactly as §1
predicted it would — and that is a parse error either way, reached by the ordinary
left-context rule with no special case for a construct that doesn't exist.

**Nothing is foreclosed.** Each is one scanner line, restored with the construct that
needs it: `..` with [ranges](#range-syntax), `::` with §13's paths.

---

## Open questions still owned by their sections

Not deferrals — these have an owner and are expected to be answered when that section is
designed. Listed here only so there's a single place to look.

| Question | Raised in | Owned by |
| --- | --- | --- |
| A total-order / bitwise-equality companion to IEEE `==` | §2 | §15 |
| Whether instance references compare by identity (provisional) | §2 | §7, §11 |
| An operator for reference identity, distinct from `==` | §6 | §11 |
| Opt-in value semantics for small structs (Rust's `Copy`) | §6 | §11 |
| What a length returns, and iteration during mutation | §6 | §8, with collections |
| Whether sorting APIs justify a three-way comparison | §2 | §8, with collections |
| Whether `+` and `==` are user-implementable | §2 | §11, with traits |
| Whether a trap is recoverable | §2 | §9, §15 |
| Whether `obj.method` without parens is a bound value | §2 | §11 |
| Whether inference needs expression-level type ascription | §2 | §10 |
| Where `mut` attaches in a destructuring pattern | §3 | §7 |
| How top-level declarations can be mutually recursive while statements run in order (sharpened by §6's sugar rule: mutually recursive types are ordinary) | §3, §6 | §12, §13 |
| Whether an uninstantiated generic can be checked at all, given bounds | §14 | §8 |
| How a comptime function rejects its arguments (`@compileError`) | §14 | §14 |
| Whether comptime evaluation may mutate during its own execution (`comptime var`) | §14 | §14 |
| Unrolled iteration over a comptime-known bound with a runtime body (`inline for`) | §14 | §14 |
| Whether type arguments may be inferred at a call site rather than passed | §14 | §10 |
| Whether `Type` values support equality, ordering, or printing at comptime | §14 | §14 |
| Where a runtime value reaching a comptime parameter is caught and blamed | §14 | §10 |
