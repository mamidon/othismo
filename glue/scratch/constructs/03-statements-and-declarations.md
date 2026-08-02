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

## Glue Semantics
