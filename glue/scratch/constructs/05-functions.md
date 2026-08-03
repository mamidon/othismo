# §5 — Functions

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox's function surface is small — first-class values, closures, nested declarations, and
arity checked at call time — and most of the cost is in the implementation rather than
the syntax. On wasm, first-class functions mean `funcref` plus a table and
`call_indirect`; closures mean heap-allocated environment objects, because wasm has
none natively; and deep recursion means trampolines until the tail-call proposal lands.

The item that matters most for Glue is the last one: **foreign / host functions**. How a
source-level declaration becomes a wasm import from the Othismo host, and how an exported
handler becomes something Othismo can call, is the seam the whole language sits on.

## Lox's grammar **[Lox]**

```
funDecl    → "fun" function
function   → IDENTIFIER "(" parameters? ")" block
parameters → IDENTIFIER ( "," IDENTIFIER )*
arguments  → expression ( "," expression )*
```

## Checklist

- **Declaration** — `fun name(params) { body }` **[Lox]**
- **First-class functions** — passed, returned, stored **[Lox]** **[wasm]** — wasm needs
  `funcref` / a table + `call_indirect`
- **Closures** — capture enclosing variables **[Lox]**, ch. 25
  - By-reference vs. by-value capture (see Design Note: Closing Over the Loop Variable, ch. 25)
  - Upvalue representation **[wasm]** — no native closures; needs heap-allocated environments
- **Nested / local function declarations** **[Lox]**
- **Arity checking** — Lox checks at call time; static checking is a typing decision
- **Parameters**
  - Default values **[Lox-omits]**
  - Named / keyword arguments **[Lox-omits]**
  - Variadics **[Lox-omits]**
  - Pass-by-value vs. reference; mutability of parameters
  - Destructuring parameters (patterns: §7)
  - Type annotations on parameters and returns — where they're required (§10)
- **Recursion**, mutual recursion, and whether forward declaration is needed
- **Tail calls** **[wasm]** — wasm has a tail-call proposal; without it, deep recursion
  needs a trampoline
- **Overloading** by arity or type (usually: don't)
- **Operator overloading** / user-defined operators
- **Generics / polymorphism** — monomorphize or box **[wasm]** → §8
- **Inline / purity / effect annotations**
- **Native / foreign / host functions** **[wasm]** — the import/export surface. For glue,
  this is arguably the *central* construct: how does a source-level declaration become a
  wasm import from the othismo host?

## Glue Syntax

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Not decided here

- **Generics** — §8.
- **Methods, receivers, and `self`** — §11. §5 covers free functions; a method adds a
  receiver and an owner, and both are §11's.
- **Host and foreign functions** — §13, because declaring one means saying where it comes
  from, which is a module-system question. This is the section's biggest absence: until
  §13 exists, a Glue program can compute but cannot observably *do* anything (§3).
- **Operator overloading** — §6, with traits.

### Declaration

```
fn add(a: u64, b: u64) -> u64 {
  a + b
}

fn log(message: Str) {
  …
}
```

- **Parameter types are required. Return type is required when it isn't unit**, written
  `-> T`, and omitted when it is.

  This is the local-inference boundary §10 wants: **signatures are annotated, bodies can
  be inferred.** Inference inside a body is available, not mandatory — annotations on a
  `let` stay optional wherever the compiler can manage without them, and permitted
  wherever a reader wants them (§3). It's also what keeps a function's meaning readable
  without reading its body, and what lets the two front ends agree without whole-program
  analysis (goal §2.2).
- The body is a block, so its value is its trailing expression (§2, §3). `return` is for
  early exit (§4) — a well-shaped function often has none.
- **No overloading**, by arity or by type. One name, one function. Overload resolution
  interacts badly with inference (§10) and with generics (§8), and the cost of `add_u64`
  versus `add_f64` is smaller than the cost of resolution rules nobody can predict.

### Parameters

- **Parameters are immutable bindings** by default, exactly like `let` (§3).
- `mut` on a parameter means **the function may mutate the caller's value**, and the
  caller must pass a `mut` binding:

  ```
  fn advance(c: mut Counter, by: u64) { … }

  let mut tally = Counter::create();
  advance(tally, 1);        // fine
  let frozen = Counter::create();
  advance(frozen, 1);       // error — frozen is not mut
  ```

  This is the mechanism §3 promised: a call that mutates requires a `mut` binding at the
  call site, so mutation is visible where it happens rather than only where it's declared.

  **Open:** whether a `mut` parameter is by reference or copy-in/copy-out. The difference
  is observable only through aliasing, which is §6's question (value versus reference
  semantics), so §5 fixes the syntax and the checking rule and leaves representation to §6.
- **No default values, no named arguments, no variadics.** All three are additive later;
  all three complicate call resolution now, and none is needed to get the language
  standing up.

### Functions as values

```
let f = add;                       // a function is a value
fn apply(g: fn(u64) -> u64, x: u64) -> u64 { g(x) }
```

The type of a function is `fn(T, …) -> R`, with `-> R` omitted for unit. On wasm this is a
`funcref` in a table, called through `call_indirect` (§16).

### Lambdas

```
let inc = |x: u64| x + 1;
let inc = |x| x + 1;               // types from context
let go  = || work();               // no parameters

items.map(|x| {
  let y = x * 2;
  y + 1
})
```

- Parameters are `|…|`; the body is an expression, which may be a block.
- **Lambda parameter and return types are inferred from context**, unlike a named `fn`.
  This is deliberate: a `fn` is a declaration that others read, a lambda is an argument
  read in place.
- `||` with no parameters is lexically identical to the logical-or operator. In expression
  position where an operand is expected, `||` opens a zero-parameter lambda; between two
  operands it is the operator. Rust has exactly this wart and it has not proved to be a
  problem in practice.

### Nested functions

A `fn` may be declared inside a block. It is scoped to that block and **captures nothing**
— it is an ordinary function that happens to be private. To capture, use a lambda.

Keeping `fn` capture-free means every `fn` compiles to a plain wasm function with no
environment, and it makes the distinction visible in the source rather than inferred from
whether a name happens to be in scope.

---

## Glue Semantics

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Closures

- **Lambdas capture by reference**, implicitly — there is no capture list. Mutation of a
  captured binding is visible to everyone holding it, and the binding must be `mut` for
  the lambda to mutate it at all (§3).
- Captured bindings therefore **outlive the frame that created them**. On wasm this means
  a heap-allocated environment, since the target has no closures of its own (§16); in the
  interpreter it means the same thing by a different route. Both back ends must agree on
  *what* is captured, which is why capture is by binding and not by expression.
- **The classic loop-variable trap is currently absent**, and worth noticing before it
  comes back. In most languages, closures created in a loop share one loop variable and
  all observe its final value. Glue has only `while` (§4), so the "loop variable" is an
  ordinary `let` inside the body — a fresh binding per iteration, captured separately.
  Whatever iteration construct §4 eventually gains inherits this question, and the answer
  should be per-iteration.

### Calls

- Arguments evaluate left to right, before the call (§2).
- Arity and types are checked statically. There is no arity checking at runtime because
  there is nothing dynamic to check.
- **Recursion is permitted.** Mutual recursion needs top-level declarations to be
  order-independent, which §12 and §13 owe an answer to (§3).
- **No tail-call guarantee.** wasm's tail-call proposal would give us one cheaply, but
  until it's relied upon, deep recursion exhausts the stack and traps (§15's resource
  limits). Code that must recurse unboundedly needs a loop or an explicit worklist.

### Parameter passing

Arguments are passed by value; a `mut` parameter additionally permits the callee to write
through to the caller's binding. What "by value" costs for an aggregate — a copy, or a
reference under the hood — is §6's, and it is the same open question as the `mut`
representation above.

### Unit

A function with no `-> T` returns unit. Unit is a real value with one inhabitant, not an
absence: it can be bound, returned, and stored, which keeps `fn` types uniform and means
generic code (§8) needs no special case for "returns nothing".
