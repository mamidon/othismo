# Deferred Decisions

> Companion register to the numbered sections. Index: [`../language-constructs.md`](../language-constructs.md)

Things consciously **postponed**, not rejected and not overlooked. Each entry records why
it was deferred, what it would cost to decide wrongly now, and what it blocks — so the
argument doesn't have to be reconstructed when it comes back up.

This register holds only postponed decisions. Questions that are actively owned by a
later section stay in that section, marked **Open**; the last part of this file points at
them so there's one place to look.

| Deferred | Raised in | Lands in | Blocks |
| --- | --- | --- | --- |
| [Range syntax](#range-syntax) | §2 | §2 | slicing (§6), iteration (§4) |
| [Pipeline operator](#pipeline-operator) | §2 | §2 | nothing; waits on §11 |
| [String interpolation](#string-interpolation) | §1 | §1 | nothing; wants §11 |
| [Multi-line indentation stripping](#multi-line-indentation-stripping) | §1 | §1 | nothing |

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

The collision-free alternative puts the markers on the operator, at a real cost in
readability:

```
a..b      →  [a, b)
a..=b     →  [a, b]
a<..b     →  (a, b)
a<..=b    →  (a, b]
```

**Nothing is foreclosed.** §1's ban on a trailing `.` in float literals keeps `..`
available as an operator whenever this is decided. Slicing (§6) and iteration (§4) both
depend on the outcome, so this is the deferral most likely to come due first.

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

A `"""` string currently takes its content literally, including every leading space on
every line. Whether it should strip indentation to match its closing delimiter is
undecided.

The feature is wanted; the exact rule is fiddly, and several languages have shipped a
version they later regretted. Since goal §3 puts Glue under no backward-compatibility
obligation during design, changing the rule later is allowed — but it *is* a change in
meaning for existing strings, which is the one reason to prefer getting it right over
getting it early.

---

## Open questions still owned by their sections

Not deferrals — these have an owner and are expected to be answered when that section is
designed. Listed here only so there's a single place to look.

| Question | Raised in | Owned by |
| --- | --- | --- |
| What integer type lengths and indices return | §1 | §6 |
| Whether `+` and `==` are user-implementable | §2 | §6 |
| A total-order / bitwise-equality companion to IEEE `==` | §2 | §15 |
| Whether instance references compare by identity (provisional) | §2 | §7, §11 |
| Whether a trap is recoverable | §2 | §9, §15 |
| Whether `obj.method` without parens is a bound value | §2 | §11 |
| How far constness propagates through immutable bindings | §1 | §10 |
| Whether inference needs expression-level type ascription | §2 | §10 |
| Whether sorting APIs justify a three-way comparison | §2 | §6 |
