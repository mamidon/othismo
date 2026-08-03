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

> In progress. Decisions are recorded here as they're settled.

### Not decided here

- **Range syntax** and the **pipeline operator (`|>`)** are deferred — see
  [Deferred decisions](deferred.md) for why, and for what each would cost. Ranges are the
  more urgent of the two: slicing (§6) and iteration (§4) both wait on them.
- **Operator overloading** is §6's to decide with traits. Until then the operators below
  are built-in and closed: they work on the types listed and no others.

### Expressions and statements

Glue has a statement/expression split (§3), but **blocks are expressions**, and so are
`if` and `match` (§7).

```
let x = if ok { 42 } else { 0 };

let y = {
  let t = f();
  t * 2
};
```

**Rust's semicolon rule** decides a block's value: the value is its trailing expression,
written without a `;`. A `;` discards.

```
{ f(); 42 }      // yields 42
{ f(); 42; }     // yields unit — the `;` discarded it
{ f(); }         // yields unit
{ }              // not a block — see §1, this is the empty map
```

`return` is unaffected: it exits the enclosing function, not the block. `if c { return 42 }
else { 0 }` returns from the function; it does not evaluate to `42`.

An `if` with no `else` has type unit, so it is usable as a statement but not as a value.
Braces are mandatory and the condition is unparenthesized (§4). A map literal in the
condition needs parens — see §1.

### Precedence

Tightest to loosest. Assignment is a statement (§3), so it does not appear here; neither
does a ternary, since `if` is an expression.

| Level | Operators | Associativity |
| --- | --- | --- |
| 1 | `f(…)` call, `a[…]` index, `.field`, `.method(…)` | left |
| 2 | `-` negate, `!` logical not, `~` bitwise complement | right (prefix) |
| 3 | `as` | left |
| 4 | `*` `/` `%` | left |
| 5 | `+` `-` | left |
| 6 | `<<` `>>` | left |
| 7 | `&` | left |
| 8 | `^` | left |
| 9 | `\|` | left |
| 10 | `==` `!=` `<` `<=` `>` `>=` | **non-associative** |
| 11 | `&&` | left |
| 12 | `\|\|` | left |

Three notes on the table, since it differs from C where C is wrong:

- **Bitwise binds tighter than comparison.** `a & b == c` is `(a & b) == c`, not
  `a & (b == c)`. C's ordering here is a historical accident that every C linter warns
  about; Rust corrected it and so do we.
- **Comparison is non-associative.** `a < b < c` is a parse error, not a type error. It
  would already fail typing (a `bool` compared to a number), but a parse error names the
  actual mistake.
- **No exponentiation operator.** `pow` is a library function, which sidesteps the
  `-2**2` precedence argument entirely.

### Operators

- **Arithmetic** — `+ - * / %` on numeric types; unary `-`.
  - Unary `-` is defined on **signed and float types only**. Negating an unsigned value
    is a type error, not a runtime trap — there's no representable result but zero, and a
    compile error says so earlier and better.
- **Bitwise** — `& | ^ ~` on integer types only, never on `bool` (that's `&& || !`).
- **Shifts** — `<<` `>>`. `>>` is arithmetic on signed types and logical on unsigned, so
  there is no third `>>>` operator; the `s`/`u` split from §1 pays for itself here.
- **Comparison** — `== != < <= > >=`, both operands the same type, result `bool`.
- **Logical** — `&& || !` on `bool` only, short-circuiting.
- **Strings** — `+` concatenates. `==` and ordering compare bytes (§1: strings are UTF-8).
- **Conversion** — `x as T`, below.

**There is no truthiness.** A condition must be `bool`. This falls out of §1 having no
`nil` and no implicit conversion, and it's why chained comparison needs no special rule.

### Conversions

```
let a: u64 = 300;
a as u32          // 300
a as u8           // traps — 300 doesn't fit
a as f64          // 300.0

a.wrapping_as_u8()      //  44
a.saturating_as_u8()    // 255
a.checked_as_u8()       // Option (§7)
```

`as` is explicit and **trapping**, deliberately unlike Rust's `as`, which truncates
silently and is the most-regretted operator in that language. Lossy conversions exist,
but they have names, so silence is never the default.

### Grouping

`( expr )` groups and nothing else — it does not change a value's type, meaning, or
evaluation. Worth stating because the interval-notation range syntax in the deferral
register is the one proposal that would have broken it.

### Declined

Considered and left out. The novelty-budget rule asks for omissions to be deliberate, so
each of these has a reason rather than an absence of one.

- **Ternary `?:`** — `if` is already an expression. Two spellings of one idea is worse
  than one spelling.
- **Exponentiation operator** — `pow` is a library function, which sidesteps the whole
  `-2**2` precedence argument.
- **`++` / `--`** — increment-as-expression is where C's sequence-point bugs live, and
  `+= 1` is one character longer.
- **Comprehensions** — `.map` / `.filter` chains cover the same ground. A comprehension
  is a second, differently-shaped way to write a loop, and it prices in at §2.3's expense.
- **Three-way comparison (`<=>`)** — needs an `Ordering` type (§7) and traits (§6) before
  it can even be written. Pairwise comparison covers nearly every use; §6 may revisit when
  sorting APIs make the case.
- **`xor` and implication keywords** — `!=` on `bool` is xor. `^` stays integer-only.
- **Type test (`is`, `instanceof`)** — narrowing is `match` (§7). A separate test operator
  is a second, non-exhaustive way to do the same thing, and exhaustiveness is the entire
  reason §7 exists.
- **Optional chaining `?.` and null-coalescing `??`** — there is no `nil` (§1). Their
  Option-shaped equivalents are §7's and §9's.
- **Expression-level type ascription (`x : T`)** — annotations live on bindings and
  parameters (§3, §5). If inference turns out to need an expression-level form, §10 says so.

### Owned elsewhere

Expression forms that will exist, but are another section's to design:

| Form | Section |
| --- | --- |
| Lambda literals; named, default, and spread arguments | §5 |
| Record and struct literals | §6 |
| `match` arms and patterns | §7 |
| `?` error propagation | §9 |
| Constructor calls — `Foo()` versus a `new` keyword | §11 |
| `sizeof` / `alignof` — only meaningful if layout is user-visible | §6 |
| Compound assignment `+=`, destructuring, parallel assignment | §3 (assignment is a statement) |
| Slicing `a[i..j]` | §6, once ranges exist |
| Bitwise intrinsics: rotates, `popcount`, `clz`, `ctz` | §6 — library functions, not operators |

The last row is worth a note: wasm has all four as instructions, so exposing them costs
essentially nothing and the interpreter can implement them directly. They're a library
question rather than a syntax one only because they don't need operators.

---

## Glue Semantics

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Evaluation order

**Left to right, everywhere, specified.** Operands of a binary operator evaluate left
then right; call arguments evaluate left to right; collection literal elements evaluate in
written order. Nothing is left unspecified for the optimizer, because goal §2.2 requires
an interpreter and a wasm compiler to produce the same observable behavior, and
"unspecified order" is exactly where two back ends diverge without anyone noticing.

`&&` and `||` short-circuit: the right operand is not evaluated when the left decides the
result. This makes them control flow wearing an operator's clothes, which is worth
remembering when §6 considers operator overloading — these two cannot participate.

### Arithmetic

- **Overflow traps.** Every arithmetic operation whose result is not representable in its
  type is an error, not a wrap. §9 decides whether a trap is recoverable; §15 records the
  taxonomy. wasm's native behavior is silent wrapping, so this costs an explicit check per
  operation — the price of not shipping C's worst default.
- **Integer division truncates toward zero.** `-7 / 2` is `-3`, matching wasm's `div_s`.
- **Remainder takes the sign of the dividend.** `-7 % 2` is `-1`, consistent with
  truncating division and with wasm's `rem_s`.
- **Division and remainder by zero trap.**
- **Shift amounts** must be non-negative and less than the width of the left operand;
  otherwise it traps. wasm masks the shift amount instead (`i32.shl` uses it mod 32),
  which silently produces a wrong-looking answer, so this is another explicit check.
- **Constant expressions are checked at compile time** rather than trapping (§1). Anything
  that would trap at runtime — overflow, division by zero, a lossy `as`, an oversized
  shift — is a compile error when all operands are constants.

### Conversions

- Integer → integer: traps unless the value is representable in the target.
- Float → integer: truncates toward zero; traps on NaN, on infinities, and on values
  outside the target's range.
- Integer → float: exact where representable, **rounds** otherwise. This matches §1's
  asymmetry — inexactness is inherent to floats and is not an error, where integer
  overflow is.
- `f64` → `f32`: rounds. There is no implicit widening in the other direction either;
  `f32` → `f64` is still written `as`.

### Equality and ordering

- **Structural for values.** Records, unions, collections, and strings compare
  field-by-field and element-by-element.
- **Identity for instance references.** A reference to a live Othismo instance compares by
  address, not by state — two actors holding equal state are emphatically not the same
  actor, and comparing them structurally would mean reading state across a message
  boundary. **Provisional:** §7 and §11 own the final word, since neither unions nor
  instance references exist yet.
- **Cross-type comparison does not exist.** `1 == "1"` is a type error, not `false`.
- **Floats follow IEEE-754.** `NaN != NaN`, and all four ordering comparisons against NaN
  are `false`. **Open:** this makes `==` non-reflexive for any value containing a float, so
  a record holding `NaN` does not equal itself. That is the correct IEEE answer and the
  wrong answer for a hash-map key or a sort. §15 needs a total-order/bitwise-equality
  companion for those uses; the operator itself stays IEEE.

### Field access and calls

`.` reads a field or calls a method; a method call is `.name(…)` with the parens. Whether
`obj.method` without parens is a first-class bound value — Lox's model — is §11's to
decide, and it interacts with whether `.` on an instance reference is a local call or a
message send. §1's `.5` rule already reserves numeric field access (`pair.0`) should §6
want it for tuples.
