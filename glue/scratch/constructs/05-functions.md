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

## Glue Semantics
