# §2 — Expressions

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Expressions are the productive core of the language: the things that have values. The
shape of this section is a precedence ladder, and the work is deciding which rungs
exist, what binds tighter than what, and which operators are worth their weight.

wasm forces two answers into the open that a tree-walking interpreter could defer:
**overflow** (wrap, trap, saturate, or promote — wasm gives wrapping `i32`/`i64` for
free) and **division by zero** (wasm traps). Both are user-visible language semantics,
not implementation details.

Precedence and associativity deserve to be written down as a full table before any of
it is implemented; it is the part most often gotten wrong.

## Lox's precedence ladder **[Lox]**

Lowest to highest binding:

```
expression → assignment
assignment → ( call "." )? IDENTIFIER "=" assignment | logic_or
logic_or   → logic_and ( "or" logic_and )*
logic_and  → equality ( "and" equality )*
equality   → comparison ( ( "!=" | "==" ) comparison )*
comparison → term ( ( ">" | ">=" | "<" | "<=" ) term )*
term       → factor ( ( "-" | "+" ) factor )*
factor     → unary ( ( "/" | "*" ) unary )*
unary      → ( "!" | "-" ) unary | call
call       → primary ( "(" arguments? ")" | "." IDENTIFIER )*
primary    → "true" | "false" | "nil" | "this" | NUMBER | STRING
           | IDENTIFIER | "(" expression ")" | "super" "." IDENTIFIER
```

## Checklist

### Operators to decide on

- **Arithmetic** — `+ - * /` binary, `-` unary **[Lox]**
  - Modulo / remainder **[Lox-omits]** — and its sign behavior on negatives
  - Integer division vs. float division; `/` on two integers
  - Exponentiation, and its associativity (right, conventionally)
  - Overflow semantics **[wasm]**: wrap, trap, saturate, or promote. wasm gives you
    wrapping i32/i64 by default and trapping integer division by zero — that's a
    user-visible language decision, not an implementation detail
  - Division by zero: trap vs. `inf` vs. error value **[wasm]**
- **Comparison** — `< <= > >=` **[Lox]**
  - Chained comparison (`a < b < c`) — allowed, banned, or (worst) silently wrong
  - Total vs. partial ordering; NaN behavior
  - Three-way comparison (`<=>`)
- **Equality** — `== !=` **[Lox]**
  - Reference identity vs. structural equality vs. user-overridable
  - Cross-type equality: is `1 == "1"` false, or a type error?
- **Logical** — `and` `or` `!` **[Lox]**
  - Short-circuiting (Lox: yes) — and the fact that this makes them control flow, not operators
  - Truthiness: which values are falsey? Lox says only `nil` and `false`. Strictly-boolean
    conditions is the other defensible answer
  - `xor`, implication (probably not)
- **Bitwise** — `& | ^ ~ << >>` **[Lox-omits]** **[wasm]**
  - Arithmetic vs. logical right shift; rotates; popcount/clz/ctz — wasm has these as
    instructions, so exposing them is nearly free
  - Shift-amount semantics when shift ≥ bit width
- **Assignment** — `=`, an *expression* in Lox **[Lox]**
  - Statement vs. expression (expression means `if (x = 1)` typos compile)
  - Compound assignment (`+=`, `-=`, `*=`, …) **[Lox-omits]**
  - Increment/decrement (`++`, `--`), prefix and postfix **[Lox-omits]**
  - Destructuring assignment (`let (a, b) = pair`)
  - Multiple/parallel assignment (`a, b = b, a`)
- **Access & call** — `f(args)`, `obj.field` **[Lox]**
  - Indexing/subscript `a[i]`, and slicing `a[i..j]` **[Lox-omits]**
  - Optional chaining `?.`, null-coalescing `??`
  - Method-call vs. field-access-returning-callable distinction (Lox: fields and
    methods share one namespace; `obj.method` is a first-class bound method)
  - Pipeline / threading operator (`|>`)
  - Range construction (`a..b`, `a..=b`)
- **Grouping** — `( expr )` **[Lox]**
- **Conditional expression** — ternary `?:` or `if`-as-expression **[Lox-omits]**
- **Type operators**
  - Cast / conversion (`as`, `x : T`) **[wasm]** — numeric conversions are explicit
    instructions in wasm; decide which are implicit
  - Type test (`is`, `instanceof`), type ascription
  - `sizeof` / `alignof` if you expose memory

### Precedence and associativity

Not a construct, but the thing most often gotten wrong:

- Write the full precedence table down before implementing
- Decide associativity per level (assignment right, arithmetic left)
- Decide whether unary minus binds tighter than exponentiation (`-2**2`)
- Prefer explicit parenthesization requirements over surprising defaults
  (see Design Note: Logic Versus History, ch. 6)

### Other expression forms worth considering **[Lox-omits]**

- Lambda / anonymous function literals (Lox has closures but no lambda syntax!)
- Block expressions (last expression is the value)
- `match` / `switch` as an expression
- Comprehensions
- `await` (relevant given othismo's async message model)
- Object/record/struct literals
- Named arguments and default arguments at the call site
- Spread / splat (`f(...args)`)
- Constructor calls: is `Foo()` a call, or is there a `new`?

## Glue Syntax

## Glue Semantics
