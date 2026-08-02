# §11 — Objects & Abstraction

> Part of the Glue construct checklist. Index and legend: [`../language-constructs.md`](../language-constructs.md)

## Summary

Lox's model is classes with single inheritance, methods, `this`, an `init()` constructor,
and `super.method()`. Fields are not declared — they spring into existence on assignment
— which is the one part most languages don't copy, because declared fields are both more
common and far more compilable.

The Othismo-specific question is the one that matters most: given a runtime whose unit is
an *instance in a namespace exchanging BSON messages*, the right declaration here may be
`actor` or `instance` with a message handler rather than `class`. Goal §2.4 wants program
structure to line up with runtime structure so that a running program is already an
addressable, inspectable graph. Goal §4.2 is the counterweight: if Glue objects map
one-to-one onto Othismo instances the result is beautifully introspectable and probably
far too slow, which implies two kinds of object — cheap in-instance ones and addressable
instance-level ones — and a language that makes that distinction visible without making
it miserable.

## Lox's grammar **[Lox]**

```
classDecl → "class" IDENTIFIER ( "<" IDENTIFIER )? "{" function* "}"
```

## Checklist

- **Class declaration** **[Lox]**, ch. 12
- **Fields** — Lox has no field declarations; fields spring into being on assignment.
  Declared fields are the more common (and more compilable) choice
- **Methods** **[Lox]** — and bound-method semantics (`var m = obj.method` keeps `this`)
- **`this` / `self`** **[Lox]** — implicit or explicit parameter
- **Constructors** — `init()` **[Lox]**, ch. 28
  - Return value rules; multiple constructors; factory functions instead
- **Single inheritance** — `class B < A` **[Lox]**, ch. 29
  - `super.method()` **[Lox]** and its resolution rules (statically, at the point of
    definition — ch. 29 is entirely about getting this right)
  - Method overriding; virtual dispatch **[wasm]** — vtables + `call_indirect`
- **Static / class methods and fields** **[Lox-omits]**
- **Getters / setters / properties** **[Lox-omits]**
- **Visibility** — public/private/module-private **[Lox-omits]**
- **Multiple inheritance / mixins / traits with default methods**
- **Prototypes instead of classes** (see Design Note: Prototypes and Power, ch. 12)
- **Abstract classes / interfaces**; sealed hierarchies
- **Composition-only** — a real option: structs + traits, no inheritance
- **Actors as a first-class construct** — given othismo's model, an `actor`/`instance`
  declaration with a message handler may belong here rather than `class`

## Glue Syntax

## Glue Semantics
