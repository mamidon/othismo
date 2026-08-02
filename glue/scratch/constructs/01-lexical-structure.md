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

## Glue Semantics
