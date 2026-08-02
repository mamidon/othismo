# §1 — Lexical Structure

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

The lexical layer decides what a token *is* — identifiers, literals, comments, and how
whitespace is treated. It is the layer of decisions that are hardest to take back,
because every later construct is spelled in terms of them, and because they are the
first thing a user sees.

The Lox baseline is deliberately spare: `ALPHA (ALPHA | DIGIT)*` identifiers, a single
number form `DIGIT+ ("." DIGIT+)?`, double-quoted strings with no interpolation, line
comments, insignificant whitespace, explicit `;`, and the literals `true` / `false` /
`nil`.

Two goals pull hardest here. §2.1 of the goals (one language from one-liner to module)
wants terse, low-ceremony surface syntax; §2.3 (boring syntax) wants conventional
spellings for everything, since the novelty budget is committed elsewhere.

## Checklist

- **Identifiers** — `ALPHA ( ALPHA | DIGIT )*`, `ALPHA` includes `_` **[Lox]**
  - Unicode identifiers? Normalization form? Case sensitivity?
  - Reserved words vs. contextual keywords
  - Sigils/naming conventions carrying meaning (`!` for effects, `?` for optional, `'a` for lifetimes)
- **Number literals** — `DIGIT+ ( "." DIGIT+ )?` **[Lox]**
  - Hex / octal / binary literals **[Lox-omits]** **[wasm]**
  - Digit separators (`1_000_000`)
  - Exponent notation (`1e10`)
  - Integer vs. float literal distinction **[wasm]** — wasm has `i32/i64/f32/f64`; a
    single "number" type means picking one and paying for it
  - Suffixes for width/signedness (`1u8`, `1i64`)
- **String literals** — `"` … `"` **[Lox]**
  - Escape sequences; which ones
  - Raw strings, multi-line strings, heredocs
  - Interpolation (`"hello ${name}"`) — this is a *parser* feature, not a lexer feature; decide early
  - Character/rune literals as a distinct type
  - Encoding: bytes vs. UTF-8 vs. UTF-16 vs. code points (see Design Note: String Encoding, ch. 19)
- **Comments** — line comments **[Lox]**
  - Block comments; nesting behavior
  - Doc comments as a distinct, machine-readable token
- **Whitespace & layout**
  - Significant indentation vs. braces
  - Statement terminators: explicit `;` **[Lox]** vs. newline-sensitive vs. implicit
    semicolon insertion (see Design Note: Implicit Semicolons, ch. 4)
  - Line continuation
- **Literals for other types**
  - Boolean `true` / `false` **[Lox]**
  - `nil` / `null` / `unit` **[Lox]** — and whether you want it at all
  - Collection literals: arrays/lists, maps/dicts, tuples, records, sets **[Lox-omits]**
  - Regex literals, date literals, unit literals (probably not, but decide)

## Glue Syntax

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Source text

- Source files are UTF-8. A leading BOM is skipped, not required.
- `LF` and `CRLF` are both accepted; `CRLF` is normalized to `LF` before lexing.
- Case-sensitive. Identifiers are ASCII, so no normalization form is needed.

### Identifiers and keywords

```
IDENT → ( ALPHA | "_" ) ( ALPHA | DIGIT | "_" )*
ALPHA → "a" … "z" | "A" … "Z"
DIGIT → "0" … "9"
```

- **ASCII only. Unicode identifiers are not planned.** Normalization forms and confusable
  homoglyphs are a security and tooling problem with nothing to show for it in a language
  whose keywords are English regardless. String literals and comments hold any UTF-8;
  identifiers do not.
- No sigils. `!`, `?`, `'`, and `-` carry no meaning inside an identifier.
- Keywords are **reserved**, not contextual. Contextual keywords buy source compatibility
  Glue doesn't need yet (goal §3: no stability obligations) and cost parser complexity
  against goal §2.2.
- Provisional reserved set, to be settled section by section as each is designed:
  `let var fn return if else while for in break continue match import export true false`

### Comments

```
// line comment
/* block comment, /* which nests */ to here */
/// doc comment — a distinct token, attaches to the following declaration (§14)
```

### Terminators and blocks

- Statements are terminated by `;`. Blocks are delimited by `{` `}`.
- Newlines are insignificant. There is no implicit semicolon insertion and no
  layout rule.
- The interactive front end may supply a missing trailing `;` on a complete line; this is
  an input convenience in the REPL, not a second grammar. A file and a prompt accept the
  same token stream.

### Number literals

```
INT     → DEC | HEX | OCT | BIN
DEC     → DIGIT ( DIGIT | "_" )*
HEX     → "0x" HEXDIG ( HEXDIG | "_" )*
OCT     → "0o" OCTDIG ( OCTDIG | "_" )*
BIN     → "0b" BINDIG ( BINDIG | "_" )*
FLOAT   → DEC "." DEC EXP? | "." DEC EXP? | DEC EXP
EXP     → ( "e" | "E" ) ( "+" | "-" )? DEC
SUFFIX  → "u8" | "u16" | "u32" | "u64"
        | "s8" | "s16" | "s32" | "s64"
        | "f32" | "f64"
```

- **A `.` is what makes it a float.** `1` is an integer literal, `1.0` is a float literal.
- A **trailing** `.` is not part of a literal: `1.` is not a float. This is what keeps
  `1..2` (range, §2) and `1.method()` unambiguous with no lookahead.
- A **leading** `.` is: `.5` is a valid float literal, equivalent to `0.5`. It needs one
  token of left context to lex — see *Lexing and left context* below — and it forecloses
  numeric field access (`pair.0`) unless that rule is applied. §6 inherits that
  constraint when it designs tuples.
- `_` may separate digits; it may not lead or trail a digit run.
- Hex, octal, and binary forms are integer-only.
- An exponent makes a literal a float even with no `.`: `1e10` is `f64`.
- A suffix names the type exactly: `255u8`, `1s32`, `1.0f32`. An unsuffixed literal gets
  its type from context — see **Glue Semantics** below.

### String and character literals

```
"…"        string; escapes processed
r"…"       raw string; no escapes
"""…"""    multi-line string; escapes processed, newlines kept
'…'        character literal — one Unicode scalar value
```

- Escapes: `\n` `\r` `\t` `\0` `\\` `\"` `\'` `\u{1F600}`.
- A `"""` string's content is taken literally, including every leading space on every
  line. No indentation is stripped.
- **Deferred, not decided:** string interpolation, and whether `"""` should strip
  indentation to match its closing delimiter. Both are additive — nothing in the lexer
  above forecloses either — and both are worth designing properly rather than in passing.

### Collection literals

```
[ ]                 empty list
[ 1, 2, 3 ]         list
{ }                 empty map
{ "k": v, "j": w }  map
```

Maps use the conventional `{key: value}`, which collides with blocks (§3). The collision
is resolved positionally:

- In **expression** position, `{` starts a map literal. `{}` there is an empty map.
- At the **start of a statement**, `{` starts a block. A map-literal expression statement
  must be parenthesized — `({"a": 1});` — which is a thing nobody writes on purpose.
- In the **header** of `if` / `while` / `for`, between the keyword and the body, a bare
  `{` is the body. A map literal there must be parenthesized. This is Go's rule for
  composite literals in control-flow headers, and it's the one users actually trip over.

Trailing commas are permitted. No set literal yet — sets wait for §6 to decide whether
they're a distinct type, and `{1, 2}` is available for them if so.

**Open — the thing that would make this hurt.** If §2 adopts block expressions (a block
whose value is its last expression), then `{` in expression position is ambiguous between
a block and a map, and the positional rule above no longer settles it. The fix is bounded
lookahead — `{` followed by `}`, or by an expression then `:`, is a map; otherwise a
block — which is implementable but is the second rule users trip over. §2 should decide
block expressions knowing it inherits this.

### Boolean and absence

- `true` and `false` are keywords.
- **There is no `nil` token.** How absence is spelled is deferred to §7, where unions and
  `Option` are designed; §1 only records that the lexer reserves nothing for it.

---

## Glue Semantics

> Decided 2026-08-02. Items marked **Open** are known gaps, not oversights.

### Integer literal typing

An unsuffixed integer literal does not have a type. It is an **unpinned integer
constant**: a mathematical integer of unbounded precision, which acquires a concrete type
only at the point it becomes a runtime value. This is Zig's `comptime_int` model.

**Constant expressions are evaluated exactly, before typing.** Arithmetic over unpinned
constants happens at compile time with unbounded precision, and the *result* is what gets
typed:

```
3 + -1        → constant 2      → u64
-3 + 1        → constant -2     → s64
(1 << 70) >> 70  → constant 1   → u64      // intermediates cannot overflow
```

That last case is a real guarantee, not a curiosity: an intermediate value in constant
arithmetic can never overflow, because there is nothing to overflow until the result is
pinned.

**Pinning** happens by this rule, in order:

1. **Context wins.** Where a specific numeric type is expected — an annotation, a
   parameter, a field — the constant takes that type. If the value doesn't fit, that is a
   compile error, not a wrap: `let x: u8 = 200 + 100` fails, it does not produce `44`.
2. **Otherwise by sign.** A non-negative value pins to `u64`; a negative value pins to
   `s64`.
3. **Otherwise a compile error.** A value fitting neither has no representation. There is
   no bignum fallback and no silent truncation.

Unary `-` is therefore an ordinary operation on constants rather than a special case in
the lexer, and `-1` needs no rule of its own.

**Immutable bindings stay unpinned.** A `let` whose initializer is a constant expression
remains an unpinned constant, so `let n = 3; n - 5` is `-2`, not an underflow. A `var`
pins immediately at its initializer, because its value can change at runtime and a type
must exist to hold it. *This is the one part of the rule I inferred rather than took from
you* — it follows from treating a `let` as a name for a constant, and it's what keeps the
`§2.1` one-liner case honest, but it is also the part with a conformance cost: the
interpreter and the compiler must agree on exactly how far constness propagates, across
REPL lines included.

**No implicit conversion between pinned types.** `u64 + s64` is a type error; so is
`u32 + u64`. Conversions are explicit, and §2 owns their spelling. This is the decision
that makes the rest of it safe — the alternative is C's promotion lattice, where mixed
comparison silently does the wrong thing.

**Runtime overflow and underflow trap.** Once values are pinned and runtime, exceeding a
type's range is an error rather than a wrap. Whether that's a trap, a recoverable error,
or something checked per-operation is §2's and §9's to settle.

The integer types are `u8 u16 u32 u64` and `s8 s16 s32 s64`; floats are `f32 f64`. The
`s` prefix (rather than `i`) matches wasm's own `s`/`u` instruction suffixes, and reads
unambiguously against `i32` meaning "32 bits, sign unspecified" in the wasm spec.

**What this fixes, and what it doesn't.** Constant folding fixes the *literal* ergonomics
— `3 + -1` and `let n = 3; n - 5` both work, and neither is a type error. It does nothing
for runtime values: `items.len() - 1` on an empty collection is still an underflow,
because `len()` is not a constant. Trapping makes that loud instead of silent, which is
the right trade, but the idiom still has to be exclusive ranges (`0..len`) and checked or
saturating operations rather than `len - 1`. **Open for §2:** what integer type the
standard library's lengths and indices return. If it isn't the same as the default, every
array boundary reintroduces mixed-sign arithmetic — the friction Swift's API guidelines
warn about.

**Constant division follows the operand kind.** Two integer constants divide as integers:
`7 / 2` is `3`. Unbounded precision makes an exact rational representable, and it is
deliberately not used; division truncates toward zero, so `-7 / 2` is `-3`, matching
wasm's `div_s`. Two float constants divide as floats: `7.0 / 2.0` is `3.5`.

**Mixing them is a build error.** `7.0 / 2` does not compile. Constant arithmetic is
homogeneous — integers with integers, floats with floats — because the alternative is an
implicit widening rule that exists only at compile time, and a language with one
conversion rule is worth more than the two characters `.0` saves. Write `7.0 / 2.0`.

Division by zero in a constant expression is a compile error, which is the one thing
constant evaluation can do that the runtime can't.

This follows from the rule that keeps folding honest: **constant arithmetic must produce
exactly what the same operation would produce at runtime on the pinned type.** Unbounded
intermediates are the single deliberate exception, and they only ever turn a runtime trap
into a working program, never a different answer. §2 owns truncation and remainder-sign
semantics for the runtime; whatever it decides, constant folding follows it rather than
having rules of its own.

### Float literal typing

Float literals follow the same shape: an unsuffixed float literal is an unpinned float
constant, pinned by context if there is any, otherwise to `f64`. A constant that cannot
be represented exactly in the pinned type is *rounded*, not rejected — unlike integers,
where the same situation is an error. That asymmetry is inherent to floats, not a
concession.

Integer and float constants do not mix. `1 + 2.0` is a build error, exactly as `u64 + f64`
is at runtime — the compile-time rule and the runtime rule are the same rule, and neither
has an exception for constants. There is likewise no implicit widening between `f32` and
`f64` (§2 owns conversions).

### Strings

- A string is a sequence of **UTF-8 bytes**, guaranteed well-formed. It is not a sequence
  of code points and not UTF-16.
- `len` is a **byte** count. Iteration by character yields Unicode scalar values.
- Slicing uses byte offsets. A slice that splits a multi-byte character is a runtime
  failure, not a silently mangled string — which category of failure is §9's to name.
- **This is the cheap answer at the boundary.** BSON strings are already UTF-8, so a
  string crossing to or from Othismo needs no re-encoding — it's a length and a pointer
  into linear memory (§6, §16).
- A `char` is one Unicode scalar value, 32 bits wide. Surrogate code points are not
  scalar values and cannot be produced by `\u{…}`.
- A string literal is a complete token. No construct nests an expression inside one —
  which is what interpolation would change, and part of why it's deferred rather than
  bolted on: it is a *parser* feature, and it needs a conversion interface (§11) that
  doesn't exist yet.

### Comments and doc comments

- Line and block comments produce no tokens and are not preserved.
- A `///` run is a token. It attaches to the declaration that follows it; a doc comment
  attached to nothing is a warning, since it almost always means a stray edit (§14).

### Lexing and left context

No lexer decision depends on parse state, with exactly one exception, stated here so it
doesn't grow quietly.

**The `.5` rule.** A `.` immediately followed by a digit begins a float literal *unless*
the preceding token could end an expression — an identifier, a literal, `)`, `]`, or `}`.
In that case the `.` is an access operator.

```
.5          → FLOAT(0.5)      no preceding token
f(.5)       → FLOAT(0.5)      preceded by "("
a + .5      → FLOAT(0.5)      preceded by "+"
pair.0      → DOT INT(0)      preceded by IDENT
(a, b).0    → DOT INT(0)      preceded by ")"
```

This is one token of left context, decidable mechanically, and unrelated to the parser's
state — it is not the JavaScript regex-versus-division problem, which needs to know what
the parser expects. It is still the only place two token streams differ by what came
before, so it gets its own conformance tests (goal §2.2).

Whitespace does not rescue an ambiguity: `pair. 0` and `pair .0` both follow the rule
above, by the preceding *token*, not by adjacency.

**The cost.** Numeric field access (`pair.0` for tuples) survives only because of this
rule. If §6 would rather spell tuple access some other way, the rule becomes unnecessary
and can be deleted — but if §6 wants `.0`, it is required.

Otherwise the lexer is a plain scanner: no mode stack, no nesting, no shared mutable
state. Both front ends goal §2.2 requires — interpreter and compiler — run the same one.
