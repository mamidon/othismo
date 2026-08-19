# §14 — Metaprogramming & Tooling Constructs

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Not core language, but genuinely constructs — and painful to retrofit, because each one
either needs a syntax slot reserved for it or a compiler pipeline shaped to accommodate
it.

Three have specific pull for Glue. Goal §2.2 (cheap to compile *and* cheap to interpret)
argues against an elaborate macro layer, since that is one of the classic ways a language
becomes miserable to compile. Goal §2.4 (inspect a running system) needs *something* at
runtime describing program structure, which is reflection under another name. And the
same goal makes test declarations more than good hygiene: an interpreter and a compiler
must be held to a shared conformance suite from the day there are two back ends.

## Checklist

- **Macros** — textual, syntactic, or hygienic; or none
- **Compile-time evaluation** (`const fn`, comptime)
- **Reflection** / runtime type info
- **Annotations / attributes / decorators** — including the ones the compiler consumes
  (`#[export]`, `#[inline]`)
- **Conditional compilation** / feature flags
- **`assert` / contracts** as syntax vs. library
- **Doc comments** and doc generation
- **Source maps / debug info** **[wasm]** — DWARF in wasm, name section
- **Test declarations** (see Design Note: Test Your Language, ch. 14 — the book is
  emphatic about this)

## Glue Syntax

> Decided 2026-08-18. Items marked **Open** are known gaps, not oversights.

### Scope of this decision

§14 has nine checklist items. This settles **compile-time evaluation**, and with it most
of §8. Attributes, conditional compilation, reflection, doc generation, debug info, and
test declarations are untouched and still unstarted.

Macros fall out decided, because choosing comptime is choosing against them. That is
recorded under *Semantics*, since a declined feature has no syntax.

### One keyword, two positions

`comptime` marks a **parameter** whose argument must be known at compile time:

```
fn max(comptime T: Type, a: T, b: T) -> T {
  if a > b { a } else { b }
}
```

and prefixes an **expression** that must be evaluated at compile time:

```
let table = comptime build_table(256);
```

There is no `comptime let`, no `comptime var`, and no `inline for`. A `let` binding is
already comptime-known when its initializer is (§3), so a binding form would name the same
thing twice — the argument §3 used to decline `const`, reaching the same answer a second
time.

### The type of a type is `Type`

`Type` is the type whose values are types. It is the only type with no runtime
representation (*Semantics*, below).

```
fn Pair(comptime A: Type, comptime B: Type) -> Type {
  struct { first: A, second: B }
}
```

`Type` is a **predeclared name, not a keyword** — the same status `u64`, `bool`, and `Str`
already have (§6). The `type` keyword stays where §6 put it, on the alias form.

**`comptime` is the only new token this section adds.** Everything else here is spelled
out of what §1 already lexes: `Type` is an identifier, an instantiation is a call, and a
type annotation is a type annotation.

### Generics have no syntax

A generic is a function with a comptime parameter. An instantiation is a call.

```
fn List(comptime T: Type) -> Type { … }

let xs: List(u64) = …;

fn first(comptime T: Type, xs: List(T)) -> T { … }
```

- **No `<>`, no `[]`, no turbofish.** §2's postfix rung already parses every one of these,
  because they are calls and nothing else.
- **The `<` ambiguity never arises.** `a < b` is a comparison, always. C++ and Rust both
  spend a disambiguation rule here; Glue has nothing to disambiguate.
- **§8's syntax slot is empty by construction.** Its semantic questions — bounds, variance,
  inference of type arguments, what a generic message handler means — are untouched and
  stay there.

### §6's declaration forms survive as sugar

**Decided 2026-08-18, jointly with §6.** Types-as-values makes both of §6's
type-introducing forms expressible as `let`. Neither is removed.

```
struct Point { x: s64, y: s64 }     ≡   let Point = struct { x: s64, y: s64 };
type InstanceId = u64;              ≡   let InstanceId = u64;
```

Keeping them costs two sugar rules and buys the conventional spellings, which is goal
§2.3's trade made in the direction §2.3 asks for: the novelty is that a type is a value,
and a reader who never needs that fact never has to see it. `struct Point { … }` still
reads the way it reads in every language that has structs.

What §14 *adds* to §6 is the anonymous form — `struct { … }` with no name, as an
expression — because a generic has to return a type it cannot name in advance. §6 owns
its identity rule; see there.

The one thing this costs is the `type` spelling. `type X = T;` keeps the keyword, so the
type of types is `Type`, matching `Str`.

---

## Glue Semantics

> Decided 2026-08-18. Items marked **Open** are known gaps, not oversights.

### One mechanism, not two

Glue takes Zig's model: **types are ordinary values at compile time, and a generic is a
function that takes or returns one.** Monomorphization is not a pass with rules of its
own; it is what calling such a function means.

§1 already committed to half of this. An unsuffixed integer literal is "an unpinned
integer constant … which acquires a concrete type only at the point it becomes a runtime
value," cited there as `comptime_int`. §14 generalizes that from one type to every type
rather than introducing a foreign idea — the two-stage story §1 tells about integers
becomes the story the whole language tells.

Stated against goal §2.3, this is the largest novelty spend outside §13. It is affordable
because it is mostly *subtraction*: no generic syntax, no separate generic type system, no
macro layer, no `const` binding form, and one new keyword in total. §8 stops being a
mechanism and becomes a list of questions about bounds and inference.

### Comptime-known

A value is comptime-known when it is

- a literal, or an unpinned constant (§1);
- a `comptime` parameter, within its function's body;
- a non-`mut` `let` whose initializer is comptime-known (§3's existing rule, unchanged);
- the result of a call whose arguments are all comptime-known, when the callee performs no
  runtime-only operation;
- the value of a `comptime` expression — which is an error when none of the above
  establishes it.

A runtime value is never comptime-known. There is no promotion, and no analysis that might
discover one.

### Evaluation is elaboration, and elaboration is not a pass

Comptime evaluation, name resolution, type checking, and monomorphization are **one
demand-driven pass**, not four in sequence. They are mutually recursive and admit no
ordering:

- an annotation `xs: List(T)` needs a comptime call evaluated before the type exists;
- evaluating that call needs `List`'s body typed;
- typing that body needs its comptime parameters bound, which is instantiation.

§10's choice of local/bidirectional inference over whole-program Hindley–Milner is what
makes this tractable. Every signature is annotated (§5), so a body can be elaborated
knowing only its own signature and the declarations it names. The decision that keeps
compilation cheap is the same one that lets typing and evaluation interleave.

### Instantiation is memoized, and that is where type identity comes from

Instantiations are cached on `(declaration, comptime argument values)`. `List(u64)`
evaluated twice yields the same instance, so **two types are the same type when they are
the same cache entry.** §6's nominal typing survives without a new rule: a generic type's
identity is its declaration plus its arguments.

The cache is also what makes monomorphization finite. A generic that instantiates itself
does not terminate without it; with it, recursion terminates as soon as arguments repeat.

### The boundary

- **Comptime to runtime.** A comptime value becomes a runtime value when its type has a
  runtime representation. `Type` does not — a type cannot be stored, passed at runtime,
  compared at runtime, or returned from a runtime function. Everything else may cross.
- **Runtime to comptime.** Nothing crosses. Ever.

That asymmetry is why the model stays cheap to compile (goal §2.2): comptime is a strictly
earlier stage, never a mutual dependency with execution.

It also *defers* §14's reflection item rather than answering it. Reflection is precisely
the question of whether `Type` acquires a runtime representation, and it can be added
later without disturbing anything above.

### Comptime is hermetic

Comptime evaluation sees pure computation over comptime-known values, plus the
declarations of the compilation unit. It sees nothing else: no image, no namespace, no
messages, no host imports, no filesystem, no clock, no randomness.

Othismo makes the opposite tempting — compilation could one day happen inside a live
image, with comptime code asking the namespace what exists. That is real and distinctive,
and it is deferred rather than declined; see
[Deferred decisions](deferred.md#comptime-access-to-the-image). Three reasons to start
closed:

- **Reproducibility.** A program whose meaning depends on image state cannot be rebuilt
  from source. That is goal §4.5's hazard at its sharpest, and inside the compiler is the
  worst place to meet it.
- **Tooling.** Deterministic comptime is what lets the language server cache
  instantiations across keystrokes. Non-deterministic comptime invalidates all of them on
  every edit.
- **Direction.** Opening this later is additive. Closing it later breaks programs.

### Comptime evaluation is bounded

Comptime is Turing-complete, so it must be able to *fail* rather than hang:

- a **fuel budget**, in evaluation steps, and
- a **recursion depth cap**, on calls and on instantiation,

each exhausted as an ordinary diagnostic naming the outermost comptime expression.

This is not robustness polish. The language server runs elaboration on half-typed programs
on every keystroke — `parser` is lossless and total for exactly that reason — and a
half-typed program is the likeliest thing in the language to loop forever.

### The cost: an uninstantiated generic is barely checked

The model's known weakness, written down rather than discovered.

A generic body is type-checked when it is instantiated. Before that it gets lexing,
parsing, and name resolution, and nothing more. `max` above is accepted as written;
whether `>` exists on `T` is answered only when someone calls it, and the diagnostic
arrives at the call site rather than at the declaration.

That is a direct tax on goal §2.4, which wants tooling to be a strength. A generic
function nobody has called yet cannot be given a red squiggle, and no amount of
language-server effort changes that.

Accepted, because the alternative — bounds strong enough to check a body ahead of
instantiation — is a second type system layered on the first, and §8 has not yet
established what a bound would be. **Open:** whether an opt-in bound buys early checking
back for the code that wants it.

### Macros are declined

Choosing comptime decides this. Compile-time evaluation covers what macros are usually
reached for — tables, specialization, generated declarations — and goal §2.2 names an
elaborate macro layer as one of the specific ways a language becomes expensive to compile.
Nothing textual, nothing syntactic, nothing hygienic.

The one thing comptime does not cover is generating declarations under *new names* from a
pattern. If that need turns up it returns as its own decision, rather than by extending
comptime sideways into the macro system that was declined on purpose.

### What reaches the back ends

Elaboration lowers to a **core IR**: typed, monomorphic, and free of comptime, generics,
and `Type`. Both back ends consume it, and it is the concrete form of goal §2.2's shared
front end.

```
source → CST → typed tree → [elaboration] → core IR → { interpreter, wasm }
```

**The CST is not rewritten.** Elaboration reads it and emits a separate representation,
because the CST is the language server's tree and has to keep the property `parser` opens
with — every byte of the source reachable from the tree. A derived node has no source
bytes to be reachable from. Core IR nodes instead carry provenance back to CST nodes, so a
diagnostic about an instantiation can name real source in both the generic and the call
that instantiated it.

Goal §2.2 asks for a conformance suite "from the first day there are two back ends." Core
IR is where it attaches: one program, one core IR, two executions, identical observable
results.

### Open

- Whether an uninstantiated generic can be checked at all, given bounds (§8).
- How a comptime function **rejects** its arguments — Zig's `@compileError`, or something
  else. Without it, a bad instantiation fails somewhere inside a body it didn't write.
- Whether comptime evaluation may mutate during its own execution (`comptime var`), which
  loops and table-building want and §3's `mut` rule does not obviously grant.
- Iteration that must unroll because the bound is comptime-known but the body is runtime
  (`inline for`).
- Whether type arguments may be **inferred** at a call site rather than passed (§10), and
  the explicit form for when inference fails.
- Whether `Type` values support equality, ordering, or printing during comptime.
- Where a runtime value reaching a comptime parameter is caught and **blamed** (§10).
