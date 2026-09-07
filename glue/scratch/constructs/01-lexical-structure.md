# §lexical — Lexical Structure

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

Two goals pull hardest here. §one-language of the goals (one language from one-liner to
module) wants terse, low-ceremony surface syntax; §boring (boring syntax) wants
conventional spellings for everything, since the novelty budget is committed elsewhere.

## Status

Legend in the [index](../language-constructs.md). *Syntax* and *Semantics* track what has
been **decided**; *Implementation* tracks what is **built** in `glue/`.

| Area | Syntax | Semantics | Implementation |
| --- | --- | --- | --- |
| Source text — UTF-8, BOM, line endings | ✓ | ✓ | ✓ |
| Identifiers and keywords | ✓ | ✓ | ✓ |
| Line and block comments | ✓ | ✓ | ✓ |
| Doc comments (`///`) | ✓ | ✓ | ✂ |
| Terminators and blocks | ✓ | ✓ | ✓ |
| Number literals — radix, separators, exponent, suffixes | ✓ | ✓ | ✓ |
| Integer literal typing — unpinned constants, pinning by context | ✓ | ✓ | ✓ |
| Float literal typing | ✓ | ✓ | ✓ |
| String and character literals, escapes | ✓ | ✓ | ✓ |
| Raw and multi-line strings | ✓ | ✓ | ✂ |
| String interpolation | — | — | — |
| Collection literals | — | — | — |
| Boolean literals; no `nil` | ✓ | ✓ | ✓ |
| Placeholder punctuation (`::`, `..`, `...`) | ✓ | · | ✂ |
| Lexing and left context | ✓ | ✓ | ✓ |

Collection literals and interpolation wait on §generics and §objects respectively; both
are deferrals rather than omissions. `## Implementation` below covers what
`glue/tokenizer` guarantees beyond the spelling.

---

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
- `LF` and `CRLF` are both accepted, and are equivalent everywhere they can appear.
- Case-sensitive. Identifiers are ASCII, so no normalization form is needed.

**The source buffer is never rewritten.** An earlier draft said `CRLF` is normalized to
`LF` *before* lexing, and that turns out to be the wrong place for it: every token's span
is a byte offset into the source, and normalizing first would leave those offsets pointing
into a string the editor doesn't have. Normalization happens instead when a `"""` literal's
*value* is decoded — the only place a line ending can survive into a value at all. The
observable semantics are the same, a decoded string always holds `LF`, and spans stay
pointed at bytes that exist on disk.

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
  Glue doesn't need yet (§non-goals: no stability obligations) and cost parser complexity
  against goal §both-modes.
- Provisional reserved set, to be settled section by section as each is designed:
  `as let mut fn struct type return if else while for in break continue match import
  export true false`

  `as` joins the list because §expressions made it the conversion operator. `for` and `in`
  are reserved despite §control declining every loop but `while`, since a reserved word
  costs nothing to hold and an unreserved one is expensive to take back.
- **Type names are not keywords.** `u64`, `f32`, `bool`, `Str`, and `char` are ordinary
  identifiers that §types happens to have bound. Reserving them would foreclose shadowing
  and buy nothing the name resolver doesn't already do.

### Comments

```
// line comment
/* block comment, /* which nests */ to here */
/// doc comment — a distinct token, attaches to the following declaration (§comptime)
//// four or more slashes — an ordinary line comment again
```

**Exactly three slashes make a doc comment.** A fourth takes it back to an ordinary
comment, so a row of slashes used as a section divider isn't a doc comment attached to
nothing. There is no block form of a doc comment; `/** … */` is a block comment.

> **Cut from the core.** Doc comments are not implemented: `///` is an ordinary line
> comment, and nothing reads documentation until §comptime exists. See
> [Deferred decisions](deferred.md#doc-comments).

### Terminators and blocks

- Statements are terminated by `;`. Blocks are delimited by `{` `}`.
- Newlines are insignificant. There is no implicit semicolon insertion and no
  layout rule.
- The interactive front end may supply a missing trailing `;` on a complete line; this is
  an input convenience in the REPL, not a second grammar. A file and a prompt accept the
  same token stream.

### Punctuation with no construct yet

Three sequences were tokens even though nothing in the language used them: `::`, `..`, and
`...`. The argument was that lexing them costs one line each and buys a better message —
without a `..` token, `0..5` lexes as `INT DOT FLOAT(0.5)` by the rule below, and the
resulting error is about a number rather than about a range.

> **Cut from the core.** The exception was worth stating and isn't worth keeping: it
> optimizes an error message for a construct nobody can write. `0..5` now lexes exactly as
> predicted above, and reaches a parse error by the ordinary left-context rule with no
> special case. Each returns with the construct that needs it. See
> [Deferred decisions](deferred.md#placeholder-punctuation).

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
- A **trailing** `.` is not part of a literal: `1.` is not a float. This keeps
  `1.method()` unambiguous with no lookahead, and keeps `..` available as an operator for
  whatever §expressions eventually decides about ranges.
- A **leading** `.` is: `.5` is a valid float literal, equivalent to `0.5`. It needs one
  token of left context to lex — see *Lexing and left context* below — and it forecloses
  numeric field access (`pair.0`) unless that rule is applied. §types inherits that
  constraint when it designs tuples.
- `_` may separate digits; it may not lead or trail a digit run. The rule is about the
  boundaries only, so `1__0` is legal — banning repeats would be a second rule for no gain.
- Hex, octal, and binary forms are integer-only. The radix prefix is lowercase; `0X1F` is
  an error naming that, rather than an integer followed by something unpronounceable.
- An exponent makes a literal a float even with no `.`: `1e10` is `f64`.
- A suffix names the type exactly: `255u8`, `1s32`, `1.0f32`. An unsuffixed literal gets
  its type from context — see **Glue Semantics** below.
- **A float suffix on an integer literal is legal**: `1f32` is `1.0f32`. The suffix names
  the type exactly, and `1` names a value that type can hold, so there is nothing to
  object to. The reverse is not: `1.0u8` is an error, because the literal's *form* already
  says float and the suffix contradicts it.
- A suffix is only a suffix when it's glued to the literal, so `255 u8` is two tokens. An
  unrecognized one — `1blah` — is still taken as part of the literal rather than split off
  as an identifier, since a number immediately followed by a name is never anything else.
- A literal too wide to represent — beyond 128 bits — is a lexical error. The unbounded
  precision below is a property of constant *arithmetic*; a literal still has to be
  written down before it can participate.

Note that the suffix rule and the radix rule interact, in a way that is not obvious and
so is worth stating: **`0x1f32` has no suffix.** `f`, `3`, and `2` are hex digits, so the
digit run swallows them. `0x1u8` does have one, because `u` isn't. This is consistent, but
it means the `f32` and `f64` suffixes are unavailable on hex literals — which costs
nothing, since those forms are integer-only anyway.

### String and character literals

```
"…"        string; escapes processed
r"…"       raw string; no escapes
"""…"""    multi-line string; escapes processed, newlines kept
'…'        character literal — one Unicode scalar value
```

> **Cut from the core.** Only `"…"` and `'…'` are implemented. One string form writes every
> program the core can express, and the other two carried a scanner arm, a diagnostic, and
> a decode path each. A consequence: with nothing able to span lines, CRLF normalization
> has nowhere to happen and is gone too. See
> [Deferred decisions](deferred.md#raw-and-multi-line-strings).

- Escapes: `\n` `\r` `\t` `\0` `\\` `\"` `\'` `\u{1F600}`.
- A `"""` string's content is taken literally, including every leading space on every
  line. No indentation is stripped.
- **Only `"""` spans lines.** A `"`, `r"`, or `'` literal ends at the newline whether or
  not its closing delimiter arrived. This is a recovery decision rather than a semantic
  one — the alternative is that a single missing quote reinterprets the rest of the file
  as a string, which is the worst thing an editor can do to someone mid-edit.
- A raw string has no escape for its own delimiter and no `r#"…"#` form, so **it cannot
  contain a `"` at all.** That's a real limitation, and the answer for now is to use an
  ordinary string. A hashed form is additive whenever something needs it.
- **Deferred:** string interpolation, and whether `"""` strips indentation to match its
  closing delimiter. Both are additive; nothing above forecloses either. See
  [Deferred decisions](deferred.md).

### Collection literals

§types has no collection types yet — a `List` or `Map` would be generic, and generics are
§generics. So there are no collection literals, and `[` and `{` are free.

This is worth stating rather than merely omitting, because of what it buys. With no map
literal, **`{` in expression position is unconditionally a block** (§expressions), and the
lookahead rule that would otherwise be needed — parse an expression, peek for `:` —
does not exist. When collections arrive, that rule arrives with them, and the choice of
delimiter can be revisited then knowing what it costs.

### Boolean and absence

- `true` and `false` are keywords.
- **There is no `nil` token.** How absence is spelled is deferred to §unions, where unions
  and `Option` are designed; §lexical only records that the lexer reserves nothing for it.

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

**Never-assigned bindings stay unpinned.** A binding remains an unpinned constant when
both of these hold, and pins at its declaration otherwise:

1. its initializer is a constant expression, and
2. it is never the target of an assignment anywhere in its scope.

So `let n = 3; n - 5` is `-2`, not an underflow — while `let n = 3; n = read(); n - 5`
pins `n` at `u64` and underflows. Both conditions are syntactic, which matters: goal
§both-modes requires the interpreter and the compiler to agree on exactly how far
constness propagates, and a rule needing type inference or dataflow to answer is a rule
the two back ends will eventually disagree about. Scanning a scope for assignments to a
name is not.

Every binding is rebindable (§statements), so the keyword cannot carry this the way
`let`-versus- `var` would in another language — condition 2 does the work instead. Across
REPL lines, a scope is the session, so restating `n` later pins it retroactively for
subsequent lines only; each line is compiled against the bindings that existed when it was
entered.

**No implicit conversion between pinned types.** `u64 + s64` is a type error; so is `u32 +
u64`. Conversions are explicit, and §expressions owns their spelling. This is the decision
that makes the rest of it safe — the alternative is C's promotion lattice, where mixed
comparison silently does the wrong thing.

**Runtime overflow and underflow trap.** Once values are pinned and runtime, exceeding a
type's range is an error rather than a wrap. Whether that's a trap, a recoverable error,
or something checked per-operation is §expressions' and §errors' to settle.

The integer types are `u8 u16 u32 u64` and `s8 s16 s32 s64`; floats are `f32 f64`. The
`s` prefix (rather than `i`) matches wasm's own `s`/`u` instruction suffixes, and reads
unambiguously against `i32` meaning "32 bits, sign unspecified" in the wasm spec.

**What this fixes, and what it doesn't.** Constant folding fixes the *literal* ergonomics
— `3 + -1` and `let n = 3; n - 5` both work, and neither is a type error. It does nothing
for runtime values: `items.len() - 1` on an empty collection is still an underflow,
because `len()` is not a constant. Trapping makes that loud instead of silent, which is
the right trade, but the idiom still has to be exclusive ranges (`0..len`) and checked or
saturating operations rather than `len - 1`. **Open for §types**, where collections are
designed: what integer type lengths and indices return. If it isn't `u64`, every array
boundary reintroduces mixed-sign arithmetic — the friction Swift's API guidelines warn
about — and §expressions forbids the implicit conversion that would paper over it.

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
into a working program, never a different answer. §expressions owns truncation and
remainder-sign semantics for the runtime; whatever it decides, constant folding follows it
rather than having rules of its own.

### Float literal typing

Float literals follow the same shape: an unsuffixed float literal is an unpinned float
constant, pinned by context if there is any, otherwise to `f64`. A constant that cannot
be represented exactly in the pinned type is *rounded*, not rejected — unlike integers,
where the same situation is an error. That asymmetry is inherent to floats, not a
concession.

Integer and float constants do not mix. `1 + 2.0` is a build error, exactly as `u64 + f64`
is at runtime — the compile-time rule and the runtime rule are the same rule, and neither
has an exception for constants. There is likewise no implicit widening between `f32` and
`f64` (§expressions owns conversions).

### Strings

- A string is a sequence of **UTF-8 bytes**, guaranteed well-formed. It is not a sequence
  of code points and not UTF-16.
- `len` is a **byte** count. Iteration by character yields Unicode scalar values.
- Slicing uses byte offsets. A slice that splits a multi-byte character is a runtime
  failure, not a silently mangled string — which category of failure is §errors' to name.
- **This is the cheap answer at the boundary.** BSON strings are already UTF-8, so a
  string crossing to or from Othismo needs no re-encoding — it's a length and a pointer
  into linear memory (§types, §wasm).
- A `char` is one Unicode scalar value, 32 bits wide. Surrogate code points are not
  scalar values and cannot be produced by `\u{…}`.
- A string literal is a complete token. No construct nests an expression inside one —
  which is what interpolation would change, and part of why it's deferred rather than
  bolted on: it is a *parser* feature, and it needs a conversion interface (§objects) that
  doesn't exist yet.

### Comments and doc comments

- **Line and block comments are tokens the grammar never sees.** An earlier draft said
  they produce no tokens at all, and that contradicts what the parser needs: a lossless
  tree, in which every byte of the source is reachable so the same tree can serve a
  formatter later. Both hold if comments and whitespace are lexed as *trivia* — real
  tokens, filtered out before the grammar looks. The invariant that buys is worth having:
  concatenating every token's text reproduces the file byte for byte, which is a property
  a test can check rather than a promise a reviewer has to keep.
- A `///` run is a token, and is **not** trivia — the parser must see it. It attaches to
  the declaration that follows it; a doc comment attached to nothing is a warning, since
  it almost always means a stray edit (§comptime). **Cut from the core** — see
  [Deferred decisions](deferred.md#doc-comments). It is the only construct that would use
  `Severity::Warning`, which is why the parser has no warnings at all today.

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
before, so it gets its own conformance tests (goal §both-modes).

Whitespace does not rescue an ambiguity: `pair. 0` and `pair .0` both follow the rule
above, by the preceding *token*, not by adjacency. Nor do comments — `pair /* c */ .0` is
field access too, since a comment is trivia and the rule looks past it to the last token
the grammar would see.

That is a stronger rule than the table suggests, and it has one consequence worth writing
down: **a literal to the left turns a following `.5` into field access however far away it
is.** `1.5 .5` lexes as `FLOAT(1.5) DOT INT(5)`, not as two floats. Nothing is lost today,
because two juxtaposed literals aren't valid syntax under any reading — but it is why the
rule is phrased in terms of "could end an expression" rather than "is an expression". The
lexer cannot know which, and the two answers must not differ.

**The cost — currently zero.** The rule exists so numeric field access (`pair.0`) can
coexist with `.5`. §types has no tuples, so nothing uses `.0` today and the rule costs
nothing; it stays to keep the option open. If tuples never arrive, `.` followed by a digit
could simply always be a float and the left-context check could be deleted.

Otherwise the lexer is a plain scanner: no mode stack, no nesting, no shared mutable
state. Both front ends goal §both-modes requires — interpreter and compiler — run the same
one.

---

## Implementation

`glue/tokenizer` implements everything above. Three properties it holds to, none of which
are visible in the syntax but all of which constrain it:

- **Total.** Lexing never fails. Malformed input produces error tokens and diagnostics,
  because the editor spends most of its time looking at half-typed programs.
- **Lossless.** Trivia are tokens and spans tile the source, so the stream reproduces the
  file exactly. Asserted over every prefix of a deliberately nasty source, since a
  truncation is how you find the scanner arm that forgot to advance.
- **Single-implementation.** Numeric literals and escape sequences each need to be
  understood twice — once to size a token, once to produce a value — and both readings go
  through one function. Two implementations would drift, and the drift would appear as the
  interpreter and the compiler disagreeing about what a literal says, which is exactly the
  risk goal §both-modes names.

Two things above are *lexical* errors rather than type errors, which is worth noting since
the boundary isn't obvious: a literal wider than 128 bits, and a float literal whose value
is infinite. Both are cases where there is no value to hand to the type checker at all.
Whether `200 + 100` fits a `u8` is not lexical, and is decided by the pinning rules above.

**Still open, and owned here:** string interpolation and `"""` indentation stripping remain
deferred (see [Deferred decisions](deferred.md)). Neither is foreclosed — interpolation is
the only one that would change the shape of the tokenizer, since a string literal is
currently a complete token with nothing nested inside it. Indentation stripping now waits
on `"""` itself, which is cut from the core.

**Implemented is narrower than decided.** Doc comments, `r"…"`, `"""…"""`, and the
placeholder punctuation above are all specified here and deliberately absent from
`glue/tokenizer`. Every one is additive; the register says what each costs and what comes
back with it.
