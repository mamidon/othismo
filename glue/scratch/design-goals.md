# Glue — Design Goals

> **Status:** Draft, 2026-07-31. This states intent and constraints, not design.
> It exists to be cited when a later decision contradicts it — either the decision
> changes, or this document does, deliberately.

## §problem — The problem

An operating system should have a language.

Windows and Linux don't. What they have instead is a cliff. On one side is the shell:
immediate, interactive, able to reach anything on the system, and built on languages
that nobody would choose for a program longer than a screen. On the other side is
"real" programming: a compiled language, a build system, a deployment step, and no
particular relationship to the operating system it runs on. The OS is a syscall
surface, not something the language knows about.

Crossing that cliff means rewriting. The shell pipeline you used to explore the
problem tells you almost nothing about how to write the program, and the program you
eventually write can't be poked at the way the pipeline could. Every tool that tries
to bridge the gap — a scripting language with a systems FFI, a systems language with
a REPL — is bolting one side onto the other rather than removing the cliff.

Othismo is a runtime, and closer in ambition to an operating system than to a
library. That makes the cliff a choice rather than an inheritance. Glue is the
language Othismo is designed around, and Othismo is the runtime Glue is designed
for. Neither is meant to be useful without the other.

## §goals — What we're optimizing for

### §one-language — One language across the whole range

The same language should serve a one-line interactive command and a long-lived
deployed application. Not two dialects, not a scripting subset — one language whose
syntax is *partial*: the small program is the large program with things left out.

Concretely, this means at minimum:

- A bare expression is a valid program. Typing an expression at a prompt evaluates it.
- Type annotations are optional wherever the compiler can manage without them.
- No mandatory ceremony — no required `main`, no module preamble, no declaration
  block before you can write the first useful thing.
- What you omit interactively is what you'd add for a program you intend to keep,
  and adding it is additive: annotations, structure, and names go *on top of* working
  code rather than requiring it be rewritten.

The test: a plausible shell one-liner and a plausible module should be recognizably
the same language, and it should be possible to grow the first into the second by
addition alone.

### §both-modes — Cheap to interpret *and* cheap to compile

Both execution modes are first-class, and neither is allowed to be embarrassing:

- **Interpreted** — for interactive use, and for evaluating code inside an already
  running system. Must start instantly. Does not need to be fast at steady state.
- **Compiled to WebAssembly** — for deployed instances. Must not require a long or
  elaborate build. Must be genuinely fast, not "fast for a scripting language."

This is a constraint on *language design*, not just implementation. Languages that
are miserable to compile got that way through their semantics: whole-program type
inference, an elaborate macro layer, semantics that only make sense with a
sophisticated runtime. Every feature gets asked whether it survives both modes.

The corresponding risk, stated plainly: two implementations of one language diverge.
Interpreter and compiler must agree on semantics, and the way to keep that true is a
shared front end and a shared conformance test suite from the first day there are two
back ends — not an intention to reconcile them later.

### §boring — Boring syntax and semantics

Deliberately unoriginal. `if`, `while`, functions, some form of basic
object-orientation, familiar operators and precedence. If a construct has a
conventional spelling, use the conventional spelling.

This isn't modesty — it's budget allocation. Novelty is a fixed resource that gets
spent on the things a user must learn before they can do anything, and Glue intends
to spend all of it in one place (§living). Every unit spent on a clever syntax is a unit
unavailable for the part that's actually the point.

See `language-constructs.md` for the checklist of constructs this implies.

### §living — Where the novelty goes: living systems

This is the actual thesis. Everything above is in service of it.

On a conventional system, a deployed application is opaque. Once compiled it can't
be inspected or changed; it only tells you what it was explicitly instrumented to
tell you. So telemetry becomes a project of its own — libraries, exporters,
collectors, a parallel system built alongside the real one, all of it hand-placed and
therefore never covering what you didn't anticipate needing.

Glue and Othismo should make a deployed system something you can reach into: inspect
its internal state, ask what it's doing, and change it, while it runs. Telemetry
should be a consequence of the system's structure rather than something added to it.

The intended mechanism is decomposition: a Glue program should fall apart naturally
into something that looks like objects passing messages. Othismo already *is* that —
instances in a namespace, addressed by path, exchanging BSON messages of the form
`/path.operation`. If Glue's units of program structure line up with Othismo's units
of runtime structure, then a running program is already an addressable, inspectable
graph, and the interesting interactions are already messages that the runtime can see,
count, trace, intercept, or record without any cooperation from the program.

Two pieces of this already exist and should be treated as commitments, not
possibilities:

- **The image.** Othismo's image is a persistent, inspectable container of modules
  and live instances (`new-image`, `import-module`, `instantiate-instance`,
  `list-objects`, `send-message`). A running system is a thing you have, not a
  process you started.
- **A telemetry slot in the wire format.** Messages already reserve
  `/othismo.telemetry` for trace context alongside operation parameters. Tracing
  isn't something to invent later; it's something to not squander.

### §lineage — Lineage

Smalltalk is the closest existing thing to the intended feel: a language and a
runtime designed together, a live image, uniform message passing, and a system you
manipulate rather than rebuild. Glue is not trying to be Smalltalk, but Smalltalk is
the standard to be measured against on liveness and introspection.

Worth studying deliberately, and for specific things:

- **Smalltalk** — the image, uniform message passing, tools built in the language they inspect
- **Erlang/OTP** — the same liveness proposition surviving contact with production;
  isolated processes, supervision, hot code loading, `observer` on a live node
- **Plan 9** — everything addressable through a per-process namespace, which is
  nearly Othismo's model and worth mining for what it got right and what it cost
- **Lisp machines** — a system that is its own development environment
- **Unix shell** — the thing to beat on immediacy, and the reason the cliff exists

## §non-goals — Non-goals

Stated as firmly as the goals, because a goals document that only accumulates is useless.

- **Not a research language.** No novel type theory, no new evaluation model, no
  contribution to the literature. Known ideas, known spellings.
- **Not a systems language.** Not competing with C, Rust, or Zig. No manual memory
  management as the primary model, no pretense of zero-cost abstraction everywhere.
- **Not maximally fast.** Fast enough that "real work" is honest — not fast enough to
  win benchmarks, and not at the price of §one-language, §both-modes, or §living.
- **Not compatible with anything.** Not a superset, subset, or transpile target of an
  existing language. Familiar, not compatible.
- **Not general-purpose in the portable sense.** Glue targets Othismo. Running well
  outside Othismo is not a requirement and shouldn't shape the design.
- **Not everything-is-a-message dogma.** The uniform-message model is a means to
  introspection, not a principle to be honored past the point where it costs more
  than it returns (see §granularity).
- **Not a stable language yet.** No backward-compatibility obligations during design.

## §tensions — Known tensions

These are unresolved, and being clear about them now is cheaper than discovering them
during implementation. They are roughly in order of how much they could hurt.

### §liveness — Liveness vs. WebAssembly

The central conflict. WebAssembly modules are immutable once instantiated. "Reach into
a deployed system and change it" is very nearly the one thing the compilation target
does not want to allow. There are approaches — keep meaningful state in the namespace
rather than in linear memory; support replacing a module and migrating its state;
ship an interpreter tier so live code can be evaluated inside a running instance;
some combination — and they have very different consequences for both language and
runtime. This one deserves its own document before much else is decided, because the
answer constrains the memory model, the module system, and what "an object" even is.

### §granularity — Message granularity

Smalltalk's uniformity was affordable because everything was one image with one
object model. Othismo has a real boundary: cross-instance messages are BSON documents
copied through host memory and routed. If Glue's objects map one-to-one onto Othismo
instances, the program is beautifully introspectable and probably far too slow. If
they don't, there are two kinds of object — cheap in-instance ones and addressable
instance-level ones — and the language has to make that distinction visible without
making it miserable. Where that line falls, and whether the programmer draws it or
the compiler does, is open.

### §telemetry — Free telemetry only covers the boundary

Runtime-observed messages give you the edges of the graph for free. They give you
nothing about what happens *inside* an instance between messages — which is where
loops, computation, and most bugs live. Either intra-instance activity is invisible,
or the language/compiler emits something, at which point it isn't free anymore. Worth
deciding what the honest claim is, and stating that rather than the stronger version.

### §cliff — Gradual typing has a performance cliff

"Omit the types and it works" plus "compiled code is genuinely fast" are in tension:
unannotated code generally means boxed, dynamically dispatched values, and wasm makes
that cost concrete. The usual outcomes are a performance cliff at the annotation
boundary and a soundness question about whether annotations are trusted or checked.
Neither is fatal, both are much cheaper to decide now than to retrofit.

### §image — The image is a mixed blessing

The image is Smalltalk's superpower and its most-cited failure: state that only exists
in the image is state that isn't in version control, isn't reviewable, and isn't
reproducible. Othismo has an image already. The relationship between source text and
image state — which is authoritative, how changes made live get captured, whether an
image can be rebuilt from source — is a question Glue can't avoid inheriting.

### §scale — Interactive convenience vs. program-scale clarity

Every affordance that makes a one-liner pleasant (implicit variables, coercions,
truthiness, terse defaults, silent failure) is a liability at ten thousand lines. The
shells got this wrong in one direction; most compiled languages refuse the question
entirely. Wanting one language for both means choosing per-feature, repeatedly, and
having a rule for how to choose.

## How to use this document

When a design decision is made, it should be traceable to a goal in §goals or an explicit
choice against one. When something in §tensions gets resolved, it moves out of §tensions
and into its own document, and this file links to it. When a goal turns out to be wrong,
edit this file and say so — a goals document that quietly stops describing the project is
worse than none.

## Related

- `language-constructs.md` — checklist of constructs a language needs, from
  *Crafting Interpreters*; the raw material §boring draws on
- `../../thoughts/namespace.md` — the namespace §living depends on
- `../../thoughts/messages.rfc.md` — message format, including `/othismo.telemetry`
- `../../thoughts/runtime.md` — host/guest boundary and the async model
