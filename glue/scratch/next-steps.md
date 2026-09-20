# Next steps — tooling, pass structure, and a corpus

> **Status:** Notes, 2026-09-08. Not decisions. This records a plan and the
> argument behind it so that picking it up later does not mean rediscovering
> the argument. Nothing here is built.

## Where things stand

`comptime` has a token and both of its positions parse. `Type` is a predeclared
name, a type is an ordinary binding, and `struct { … }` is an expression whose
value is a type — so `struct Point { … }` and `let Point = struct { … };` lower
to the same IR. §comptime's boundary is enforced: `Type` has no runtime
representation, and every position that would put one in a slot, global, field,
parameter, or return is refused, so core IR still contains none.

What is **not** built is evaluation. An instantiation is parsed and refused:
"generic instantiation is not supported yet". Making it work needs a comptime
configuration of `eval` and a cache keyed on `(declaration, comptime
arguments)`.

That is the point this plan is about — not because the remaining work is large,
but because it is the first thing that makes elaboration *re-entrant*, and the
current pass structure does not obviously survive that.

---

## The plan

Three items, in this order. 1 unblocks 3, and 3 is what makes 2 safe to attempt.

### 1. A CLI that can dump any stage

One tool that takes Glue source and prints what any stage made of it — tokens,
CST, whatever IRs exist, wasm once there is any, or the evaluated value.

Today this is split across two binaries: `glue` (runs a file) in `interpreter`,
and `glue-ir` (dumps core IR) in `elab`. They should be one.

Shape that worked when I spiked it:

```
glue [--emit STAGE] [FILE]

  --emit STAGE     stop after STAGE and print what it produced
  --list-stages    print the stages this build can emit, one per line
```

Four properties worth keeping, each of which earns its place in item 3:

- **Artifact on stdout, diagnostics on stderr**, so a harness can capture them
  apart.
- **`--list-stages` is machine-readable.** A harness enumerates stages rather
  than hard-coding them, so a `zir` stage appearing in the middle is covered
  the day it is added rather than the day someone remembers.
- **A stage emits even when earlier stages complained.** A poisoned IR is
  usually the thing you wanted to look at. `value` is the exception — running a
  program elaboration reported on means guessing what it meant.
- **Stages are named in pipeline order** in one place, so the help text, the
  listing, and the corpus cannot disagree.

**Also worth deciding here:** the crate holding the driver is still called
`interpreter`, and it is not the interpreter any more — `eval` is. Renaming it
`cli` is cheap; it is a leaf and nothing depends on it.

### 2. Redesign the pass data structures

This is the ZIR question, and it is the item with real design content.

**Zig's shape.** AstGen lowers the AST to **ZIR** — untyped, desugared, names
already flattened to operand indices, produced once per file. **Sema** then
interprets ZIR with a comptime environment and emits **AIR**, which is typed and
monomorphic. Type checking, inference, and comptime evaluation are all Sema, and
Sema re-runs over the same ZIR once per instantiation.

**Core IR is already AIR.** Typed, monomorphic, comptime-free. Nothing built so
far is invalidated by this; the proposal is a layer *above* it.

```
source → CST ──AstGen──→ ZIR ──Sema ⇄ eval──→ core IR → { interpreter, wasm }
                    untyped,                   typed,
                    resolved,                  monomorphic
                    desugared,
                    no trivia
```

**The argument for it**, strongest first:

- **Name resolution is instantiation-independent; typing and evaluation are
  not.** `A` in `Pair`'s body always resolves to "the first parameter of
  `Pair`" — every instantiation, forever. What varies is that it is `u64` this
  time and `Str` next. Same for desugaring. Re-walking the CST per
  instantiation redoes work that provably cannot change. That is a real seam,
  not a performance nicety.
- **It deletes a re-entrancy problem rather than solving it.** `Lowerer`
  carries `scopes: Vec<Scope>` and resolves names by walking it backwards
  (`elab/src/lower.rs`). To instantiate `Pair` from inside `area`'s body,
  elaboration would have to suspend `area`'s scope stack, rebuild the one live
  at `Pair`'s declaration, walk, and restore — while `self.funcs` is also
  mid-flight. AstGen compiles the scope chain away into operand indices, so
  Sema is a function of `(ZIR index, comptime environment)` with no ambient
  lexical state at all.
- **`lower.rs` is 2,900 lines** doing name resolution, type checking, ANF,
  capture analysis, and slot allocation in one pass. The split falls along a
  line that already exists inside it.
- **ZIR is cacheable per file**, which is the incremental story goal §living
  wants from the language server.

**What it does not buy.** ZIR is untyped, so an uninstantiated generic still
gets no type checking — §comptime's documented weakness stays exactly as
documented, and Zig has the same hole. Expect name-resolution errors inside an
uncalled generic and nothing more.

**Scope.** Glue's ZIR should be far smaller than Zig's several hundred
instructions. No `for`, no error unions, no `defer`, no `orelse`, no async, no
`inline for`, no `comptime var` — declined or deferred. Perhaps forty. Zig's
Sema is famously the hardest part of that compiler and most of the reasons do
not apply here.

**Open, and the thing to settle first:** whether to do this before or after
making instantiation work by re-walking the CST. Re-walking is cheaper to build
and hits the re-entrancy problem head-on; the split is more work and makes the
problem not exist. Doing it *before* means designing ZIR against a comptime
implementation that does not exist yet.

### 3. An end-to-end corpus, independent of the implementation

A body of Glue with expected output per pass, as plain files rather than Rust.
Current coverage is decent but every test is written against a crate API, so it
moves whenever the crates move.

Shape that worked:

```
corpus/
  <case>/
    input.glue
    tokens.expected
    cst.expected
    ir.expected
    value.expected
```

An expected file holds the stage's stdout; if the stage also reported
diagnostics they follow a `--- diagnostics ---` line, with the input path
rewritten to `input.glue` so files are not machine-specific. The harness shells
out to the `glue` binary, which is what makes the cases survive `elab` being
split. `UPDATE_EXPECT=1` regenerates.

**Two things to decide before writing it:**

- **This makes `ir::dump` load-bearing.** `core-ir.md` says core IR has no
  syntax and any textual rendering is "a debugging aid with no normative
  status". A corpus asserting on the dump changes that unless it is explicitly
  framed as a *change detector* rather than a specification — recording what the
  implementation does so that changing it is a decision rather than an
  accident. The rules stay in the unit tests, where the section each comes from
  is named.
- **`ir.expected` is brittle**, because slot numbering and temporary names move
  when lowering changes. That is partly the point, but it argues for small
  cases about one construct each, and against dumping whole example files.

---

## Two things the spike turned up

I built items 1 and 3 to see the shape, then reverted them. Two findings are
worth keeping, and both are independent of whether any of the above happens.

### Elaboration diagnostics are not in source order

`interpreter/src/main.rs` says it reports them "in source order, because a
person fixing a file wants the list rather than the first line of it". They are
not. `fn` bodies are lowered at the end of their block rather than at their
position (`elab/src/lower.rs`, the `NodeKind::FnDecl => {}` arm in `stmt`), so a
mistake inside the first function in a file is reported after every
statement below it:

```
input.glue:2:14: expected `Str`, found `an integer constant`
input.glue:3:9: integer and float constants do not mix …
input.glue:4:1: comptime is not supported yet
input.glue:1:27: no binding named `missing` is in scope     ← line 1, reported last
```

`interpreter::syntax_errors` already sorts lexical and grammatical diagnostics
by span for exactly this reason. The fix is a sort at the reporting site, not
in `elab` — emission order is what the elaboration tests pin, and it is the
more useful order for debugging the pass itself.

### The CST renderer

`Tree::dump()` is one line and is what the parser tests compare. Reading a
whole file's tree needs an indented rendering with token text beside each node,
which does not exist. Small, and item 1 wants it.
