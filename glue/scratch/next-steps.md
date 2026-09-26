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

Two items. 2 is what makes 1 safe to attempt, so 1 is written down first and
built second.

### 1. Redesign the pass data structures

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

### 2. An end-to-end corpus, independent of the implementation

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
