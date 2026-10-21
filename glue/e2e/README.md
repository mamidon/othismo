# The end-to-end corpus

Glue programs, with what each stage of the compiler made of them written down
beside them. The harness is `cli/tests/e2e.rs`; it shells out to the `glue`
binary rather than calling any crate's API, which is what makes these cases
survive the pipeline being rearranged underneath them.

```
cargo test -p cli --test e2e
UPDATE_EXPECT=1 cargo test -p cli --test e2e    # regenerate expectations
E2E_FILTER=struct cargo test -p cli --test e2e  # only matching scenarios
```

## Layout

```
<scenario_snake_case_name>/
  inputs/main.glue        the program
  ir/main.sexpr           `glue dump inputs/main.glue ir` on stdout
  ir/main.stderr          …and its diagnostics, when it has any
  output.txt              `glue eval inputs/main.glue` on stdout
  output.stderr           …and its diagnostics, when it has any
```

One directory per stage, named as `glue dump` names it: `tokens`, `cst`, `ir`,
`value`. An expectation file is named after the input it belongs to, so a
scenario with `inputs/first.glue` and `inputs/second.glue` has
`ir/first.sexpr` and `ir/second.sexpr` — and `output/first.txt` in place of
`output.txt`, since Glue has no imports and each file is its own program.

Two rules carry most of the weight:

- **An absent file asserts nothing.** A scenario pins a stage when the
  directory for it exists. Most scenarios pin `ir` and `output.txt` only;
  `tokens` and `cst` are pinned where the syntax is the point, because they are
  long and they move for reasons unrelated to what a scenario is about.
- **An absent `.stderr` claims the run was silent.** That is worth asserting:
  a program that begins reporting something it used to compile cleanly is the
  regression this corpus exists to catch.

An empty `output.txt` is therefore a real claim — "printed nothing" — which is
what a program that trapped or failed to elaborate does.

## Adding a scenario

```
mkdir -p e2e/my_scenario/inputs
$EDITOR e2e/my_scenario/inputs/main.glue
UPDATE_EXPECT=1 cargo test -p cli --test e2e
```

A scenario with no expectations yet gets `ir` and `output.txt`; to add another
stage, `mkdir e2e/my_scenario/cst` and regenerate. To drop one, delete the
directory.

## What this is and is not

A **change detector**, not a specification. `core-ir.md` says core IR has no
syntax and any textual rendering of it is "a debugging aid with no normative
status" — that is still true, and a corpus asserting on the dump does not make
the dump normative. What it does is record what the implementation produces, so
that changing it is a decision rather than an accident. Read the diff before
regenerating.

What the language *requires* is pinned in the unit tests, where the section of
the construct checklist each rule comes from is named. Scenarios here are small
and about one construct each, because `ir` expectations move when slot
numbering or temporary naming moves.
