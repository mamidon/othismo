# §3 — Statements and Declarations

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Whether the language has a statement/expression split at all is the most structural
syntax decision available. Lox has one; many languages don't, and make everything an
expression instead.

This section is where goal §2.1 bites hardest: "a bare expression is a valid program"
and "no required `main`, no module preamble" are both claims about what the top level of
`program` accepts.

## Lox's grammar **[Lox]**

```
program     → declaration* EOF
declaration → classDecl | funDecl | varDecl | statement
statement   → exprStmt | forStmt | ifStmt | printStmt | returnStmt | whileStmt | block
```

## Checklist

- **Statement/expression split** — Lox has one; many languages don't. This is the single
  most structural syntax decision (see Design Note: Expressions and Statements, ch. 3)
- **Expression statement** — `expression ";"` **[Lox]**
  - Rule for discarding non-unit values: silent, warning, or requires explicit discard
- **Block** — `"{" declaration* "}"` **[Lox]**, introduces a scope
- **Variable declaration** — `var IDENTIFIER ( "=" expression )? ";"` **[Lox]**
  - Default-initialize to `nil` **[Lox]** vs. require initializer vs. definite-assignment analysis
  - Mutable vs. immutable bindings (`let` / `const` / `var`)
  - Shadowing rules: allowed in inner scope, banned in same scope
  - Implicit declaration on first assignment (see Design Note: Implicit Variable Declaration, ch. 8)
  - Type annotations
  - Declaration *statements* vs. only-in-blocks: Lox bans `if (x) var y = 1;` by
    splitting `declaration` from `statement` — a subtle but load-bearing grammar trick
- **Constants** — compile-time constants, and whether they're a separate construct
- **`print` statement** **[Lox]** — a deliberate crutch so Lox needs no stdlib. In a real
  language this is a library function, not syntax. Decide what your equivalent bootstrap is
  (for a wasm guest: probably a host import)

## Glue Syntax

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### The statement/expression split

Glue has one, with §2's carve-out: blocks, `if`, and `match` are expressions. What remains
a statement is everything that binds a name or performs an effect in sequence.

```
statement  → exprStmt | declStmt | assignStmt | controlStmt
exprStmt   → expression ";"
declStmt   → "let" "mut"? pattern ( ":" type )? "=" expression ";"
assignStmt → place assignOp expression ";"
```

**Lox's declaration/statement split is unnecessary here.** Lox separates the two
productions purely to reject `if (x) var y = 1;`. Braces are mandatory in Glue (§4), so
that program is already unwriteable — `if x { let y = 1; }` is the only spelling, and it
scopes correctly on its own. One less grammar rule for the same guarantee.

### Bindings

```
let n = 42;              // binding
let n: u32 = 42;         // with annotation
let mut count = 0;       // mutable binding
```

- **One keyword.** There is no `var` and no `const`. A binding is `let`, optionally `mut`.
- **`mut` gates mutation, not rebinding.** Every binding is rebindable regardless; `mut`
  decides only whether the value can be changed in place. See *Mutation* below.
- **An initializer is always required.** There is no declare-then-assign, and no
  definite-assignment analysis to specify or implement. Blocks being expressions makes
  this painless — the case that would otherwise need it is written as a value:

  ```
  let mode = if verbose { Mode.Loud } else { Mode.Quiet };
  ```

- **No implicit declaration.** Assigning to an unbound name is an error, never a
  declaration.
- **The left side is a pattern** (§7), so destructuring falls out of `let` rather than
  needing its own form. Until §7 exists, the only pattern is a plain name. Patterns in
  `let` must be irrefutable.

### Constants

**There is no `const` keyword.** §1 already gives the useful half: a binding with a
constant initializer that is never assigned to stays an unpinned constant, folded at
compile time and usable wherever a constant is expected. A second keyword would name the
same thing twice.

What that does *not* provide is a way to **require** compile-time evaluation — the thing
you need if an array length or a type parameter must be a constant. That requirement can't
be stated until there's a construct that needs it, so §6 introduces it if and when array
sizes do. Adding a `const` binding form later is additive; retrofitting one that means
something subtly different from `let` is not, which is the reason not to guess now.

### Shadowing

A new `let` may shadow an existing binding, **including in the same scope**:

```
let input = read();          // Str
let input = parse(input);    // u64 — same name, new binding
```

This is deliberate and it earns its keep twice. It's the natural way to write a
narrowing pipeline without inventing `input2`, and it's what makes redefinition at a
prompt behave the way anyone would expect — goal §4.5 wants a live session where you
restate a binding, and shadowing is that, with no special REPL rule.

Shadowing creates a *new* binding; it does not mutate the old one. Anything that captured
the old binding still sees the old value (§12).

### Assignment

Assignment is a **statement**, so `if x = 1 { … }` does not compile — the typo class is
gone by construction rather than by lint.

```
count = count + 1;
count += 1;
```

Compound forms: `+= -= *= /= %= &= |= ^= <<= >>=`. Each is exactly `a = a op b` with the
place evaluated once, and each traps on overflow like the operator it wraps (§2).

The left side is a **place**: a name, a field, or an index — not a pattern. Parallel
assignment (`a, b = b, a`) is declined; `let` destructuring covers the real need, and a
statement that assigns to several places at once interacts badly with evaluation order
being fully specified (§2).

### Expression statements

An expression statement evaluates its expression and discards the value.

```
log("hello");
counter.next();        // whatever it returns is discarded
```

No rule requires the value to be unit, and nothing marks a discard as deliberate.

### The top level

**A file is a block.** It holds statements, and it may end in a trailing expression with
no `;`, which is the file's value under §2's rule.

```
let x = 2;
x * 21          // the value of this file: 42
```

That is the whole of goal §2.1's "a bare expression is a valid program" — not a REPL
special case, just the block rule applied to the outermost block. A prompt and a file
accept the same input because they *are* the same construct.

There is no `print` statement. Output is a host import (§13); the interactive front end
displays the top-level trailing expression, which is a property of the language rather
than a feature of the REPL. What a *deployed* module does with a top-level value — and
whether it has one — is §13's, along with the entry point.

Module-level bindings, `mut` included, are permitted; when they initialize and in what
order is §12's and §13's.

---

## Glue Semantics

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Scope and lifetime

- A block introduces a scope (§12). A binding is visible from its declaration to the end
  of the enclosing block — not before, which makes use-before-declaration a static error
  rather than a runtime surprise.
- Shadowing introduces a new binding. The shadowed one is not destroyed, merely
  unreachable by name; a closure that captured it (§5) still sees it.
- **Open:** whether a `let` at module scope is visible to declarations above it. Functions
  need to be mutually recursive, so *some* top-level forms must be order-independent while
  statements are order-dependent. §12 and §13 own the rule; §3 records that the two cases
  cannot both be "in order."

### Initialization

Every binding is initialized at its declaration, so there is no uninitialized state to
observe and no default-value rule to write. This is a direct consequence of §1 having no
`nil`: without a universal absent value, "declared but unset" has nothing to hold.

### Mutation

`mut` gates **in-place mutation only**. Rebinding is always allowed, on any binding. There
is no type-level `mut` qualifier.

```
let x = Foo::create();
x.mutating_method();     // error — mutates, and x is not mut
x = Foo::create();       // fine — rebinding is unrestricted

let mut y = Foo::create();
y.mutating_method();     // fine
y = Foo::create();       // fine
```

The two operations are genuinely different, which is why one keyword covers only one of
them: rebinding replaces what a *name* refers to and can affect nothing else, while
mutation changes a value that other names may also observe. Only the second needs
guarding.

Assignment must match the binding's type — `x = v` requires `v` to have `x`'s type. To
give a name a value of a different type, declare it again; that's shadowing, above, and it
is why both forms remain useful.

This rests on a function being able to declare that it mutates its receiver or a
parameter, which is §5's and §11's to design. §3 fixes only the rule that consumes it:
calling such a function requires a `mut` binding.

**Open:** where `mut` attaches once patterns exist — `let mut (a, b) = …` marking the
whole binding, or `let (mut a, b) = …` marking each. §7 decides when destructuring does.

**What this does not give us.** Rust's version carries a real guarantee because borrow
checking forbids a second, mutable path to the same value. Glue is not adopting ownership
or borrowing (goal §3), so `let` means "you cannot mutate through *this* name" — not "this
value will not change". If §6 gives aggregates reference semantics, then

```
let a = obj;
let mut b = obj;
b.mutate();        // a observes the change
```

is possible, and `let` is an ergonomic guard rather than a guarantee. If §6 chooses value
semantics, the situation can't arise. **Open for §6** — and the answer decides how strong
a claim the documentation is allowed to make about `let`.

### Evaluation

Statements execute in written order. The initializer of a declaration is evaluated before
the binding exists, so `let x = x;` refers to the *outer* `x` if there is one, and is an
error if there isn't — which is what makes the shadowing idiom above work.
