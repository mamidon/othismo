//! Tests for elaboration.
//!
//! Organized by the section each rule comes from, because that is where the
//! argument for it lives and where a change to it would be made. A test that
//! checks a whole dump is checking the *shape* of the IR; one that checks a
//! diagnostic is checking a rule.

use crate::{DiagnosticKind, Program, dump, lower};

fn lowered(source: &str) -> (Program, Vec<String>) {
    let parse = parser::parse(source);
    assert!(
        parse.diagnostics.is_empty(),
        "the test source did not parse: {:?}",
        parse
            .diagnostics
            .iter()
            .map(|d| d.message())
            .collect::<Vec<_>>()
    );
    let lowered = lower(&parse.tree, source);
    let messages = lowered
        .diagnostics
        .iter()
        .map(|d| d.message())
        .collect::<Vec<_>>();
    (lowered.program, messages)
}

#[track_caller]
fn ir(source: &str) -> String {
    let (program, errors) = lowered(source);
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
    dump(&program)
}

#[track_caller]
fn errors(source: &str) -> Vec<String> {
    let (_, errors) = lowered(source);
    assert!(!errors.is_empty(), "expected a diagnostic, got none");
    errors
}

#[track_caller]
fn only_error(source: &str) -> String {
    let errors = errors(source);
    assert_eq!(errors.len(), 1, "expected one diagnostic: {errors:#?}");
    errors.into_iter().next().unwrap()
}

// ---- The shape of it -------------------------------------------------------

/// Goal §one-language: a bare expression is a whole program, because
/// §statements makes a file a block and a block's value is its trailing
/// expression.
#[test]
fn a_file_is_a_block() {
    assert_eq!(
        ir("42"),
        "\
(func <file> () -> u64
  (block 0
    (return (const 42u64))))   ; the file's top level (§statements)"
    );
}

/// The worked example from `scratch/core-ir.md`. It exercises ANF's loop
/// header, slot reuse across iterations, and the peephole that keeps
/// `i = i + 1` from emitting a dead copy.
#[test]
fn loops_slots_and_temporaries() {
    let source = "\
fn count_to(n: u64) -> u64 {
  let mut i = 0;
  while i < n * 2 {
    i = i + 1;
  }
  i
}";
    assert!(ir(source).contains(
        "\
(func count_to (u64) -> u64
  (slot 0 n  u64 param)
  (slot 1 i  u64 local mut)
  (slot 2 t2 u64 temp)
  (slot 3 t3 bool temp)
  (block 0
    (assign i (const 0u64))
    (while
      (header 1
        (assign t2 (mul n (const 2u64)))
        (assign t3 (lt i t2)))
      (cond t3)
      (body 2
        (assign i (add i (const 1u64)))))
    (return i)))"
    ));
}

/// The second worked example: a captured binding that is assigned needs a
/// heap cell, because a slot dies with its frame (§functions).
#[test]
fn captured_and_assigned_gets_a_cell() {
    let source = "\
fn counter() -> fn() -> u64 {
  let mut n = 0;
  () -> { n = n + 1; n }
}";
    let ir = ir(source);
    assert!(ir.contains(
        "\
(func counter () -> (fn () -> u64)
  (slot 0 n  (cell u64) local mut)
  (slot 1 t1 (fn () -> u64) temp)
  (block 0
    (assign n (makecell (const 0u64)))
    (assign t1 (closure counter.λ0 (captures n)))
    (return t1)))"
    ));
    assert!(ir.contains(
        "\
(func counter.λ0 () -> u64
  (slot 0 n  (cell u64) capture mut)"
    ));
    assert!(ir.contains("(store (cell n) t2)"));
}

/// A captured binding that is never assigned is copied into the environment
/// instead. Every copy stays equal forever, so the sharing a cell provides is
/// unobservable — and under §types' reference semantics a struct binding
/// copies its reference, so mutation of the object is visible either way.
#[test]
fn captured_but_never_assigned_needs_no_cell() {
    let ir = ir("\
fn adder(n: u64) -> fn(u64) -> u64 {
  (x) -> x + n
}");
    assert!(ir.contains("(slot 0 n  u64 param)"), "{ir}");
    assert!(!ir.contains("makecell"), "{ir}");
    assert!(ir.contains("(captures n)"), "{ir}");
}

/// A `mut` binding nobody captures is an ordinary slot. Cells are for crossing
/// a frame boundary, not for mutability.
#[test]
fn mut_without_capture_needs_no_cell() {
    let ir = ir("let mut n = 0u64; n = n + 1; n");
    assert!(!ir.contains("makecell"), "{ir}");
}

/// Invariant 2: every operand is a slot or a constant, so a nested expression
/// becomes a sequence and §semantics' evaluation order is the order of the
/// data.
#[test]
fn operands_are_atomic() {
    let ir = ir("fn f(a: u64, b: u64, c: u64) -> u64 { (a * a) - (b * c) }");
    assert!(
        ir.contains(
            "\
    (assign t3 (mul a a))
    (assign t4 (mul b c))
    (assign t5 (sub t3 t4))"
        ),
        "{ir}"
    );
}

// ---- §lexical Unpinned constants -------------------------------------------

/// §lexical: a binding stays an unpinned constant when its initializer is one
/// and it is never assigned, so this is `-2` rather than an underflow.
#[test]
fn an_unassigned_constant_binding_stays_unpinned() {
    assert_eq!(
        ir("let n = 3; n - 5"),
        "\
(func <file> () -> s64
  (block 0
    (return (const -2s64))))   ; the file's top level (§statements)"
    );
}

/// §lexical's other half: assignment anywhere in the scope pins the binding at
/// its declaration. The subtraction is then an ordinary runtime one on `u64`,
/// which §expressions says traps on underflow rather than folding to a
/// negative.
#[test]
fn an_assigned_binding_pins_at_its_declaration() {
    let ir = ir("let n = 3; n = 4; n - 5");
    assert!(ir.contains("(global @n u64)"), "{ir}");
    assert!(ir.contains("(sub t0 (const 5u64))"), "{ir}");
}

/// §lexical: context wins over sign.
#[test]
fn context_pins_a_constant() {
    assert!(ir("let x: u8 = 200; x").contains("(const 200u8)"));
}

/// §lexical: an integer may carry a float suffix.
#[test]
fn an_integer_may_carry_a_float_suffix() {
    assert!(ir("1f64").contains("(const 1.0f64)"));
}

/// §lexical: no implicit conversion between pinned types. There is no
/// promotion lattice, so mixed widths are an error rather than a silent
/// widening.
#[test]
fn pinned_types_do_not_mix() {
    assert_eq!(
        only_error("fn f(a: u32, b: u64) -> u64 { a + b }"),
        "`+` needs both operands to have the same type, and these are `u32` and `u64` — \
         conversions are explicit"
    );
}

// ---- §expressions ----------------------------------------------------------

/// §expressions: constant expressions are checked at compile time rather than
/// trapping.
#[test]
fn constant_failures_are_compile_errors() {
    assert_eq!(
        only_error("let x: u8 = 200 + 100; x"),
        "the constant 300 does not fit in `u8`"
    );
    assert_eq!(
        only_error("1u64 / 0"),
        "this constant expression divides by zero"
    );
}

/// §expressions' rule reaches *pinned* constants too. `255u8 + 1` has a width
/// to overflow, so it is an error rather than the `0` wrapping would give.
#[test]
fn pinned_constants_are_checked_too() {
    assert_eq!(
        only_error("255u8 + 1"),
        "this constant expression overflows — constants are checked at compile time"
    );
    assert_eq!(
        only_error("1u64 / 0"),
        "this constant expression divides by zero"
    );
    assert!(ir("254u8 + 1").contains("(const 255u8)"));
}

/// §expressions: `+` concatenates strings, and folding one is the same rule.
#[test]
fn constant_strings_concatenate() {
    assert!(ir("\"a\" + \"b\"").contains("(const \"ab\")"));
}

/// §lexical: an intermediate in constant arithmetic can never overflow,
/// because there is nothing to overflow until the result is pinned.
#[test]
fn constant_intermediates_do_not_overflow() {
    assert!(ir("(1000000 * 1000000) / 1000000").contains("(const 1000000u64)"));
}

/// §expressions: there is no truthiness. A condition is a `bool` or it is an
/// error.
#[test]
fn a_condition_must_be_bool() {
    assert_eq!(
        only_error("if 1u64 { 2u64 } else { 3u64 }"),
        "a condition must be `bool`, and this is `u64` — there is no truthiness"
    );
}

/// §expressions: negating an unsigned value is refused at compile time,
/// because there is no representable result but zero and an error says so
/// earlier and better.
#[test]
fn unsigned_negation_is_a_type_error() {
    assert_eq!(
        only_error("fn f(x: u64) -> u64 { -x }"),
        "`u64` is unsigned, so there is no value for `-` to produce"
    );
}

/// §expressions: `&&` and `||` short-circuit, so they are control flow and
/// lower to `if`. Neither back end implements laziness, and `BinOp` has no
/// entry.
#[test]
fn short_circuit_lowers_to_a_branch() {
    let ir = ir("fn f(a: bool, b: bool) -> bool { a && b }");
    assert!(ir.contains("(if t2"), "{ir}");
    assert!(!ir.contains("(and "), "{ir}");
}

/// §expressions: cross-type comparison does not exist — `1 == \"1\"` is a type
/// error and not `false`.
#[test]
fn cross_type_comparison_is_an_error() {
    assert!(only_error("1u64 == \"1\"").contains("needs both operands to have the same type"),);
}

/// §expressions: `as` is explicit. Float to integer truncates toward zero,
/// which is defined behaviour rather than a trap, so it happens at compile
/// time.
#[test]
fn constant_casts_follow_the_conversion_table() {
    assert!(ir("1.5 as s64").contains("(const 1s64)"));
    assert!(ir("300 as f64").contains("(const 300.0f64)"));
    assert_eq!(
        only_error("300 as u8"),
        "the constant 300 does not fit in `u8`"
    );
}

// ---- §statements -----------------------------------------------------------

/// §statements: a `let` may shadow an existing binding, including in the same
/// scope. Two bindings means two slots, and the dump disambiguates them.
#[test]
fn shadowing_makes_a_second_binding() {
    // At the top level that is a second *global* (§statements).
    let top = ir("let x = 1u64; let x = x + 1u64; x");
    assert!(top.contains("(global @x.0 u64)"), "{top}");
    assert!(top.contains("(global @x.1 u64)"), "{top}");

    // Inside a block it is a second slot, which is the same rule against
    // different storage.
    let scoped = ir("{ let x = 1u64; let x = x + 1u64; x }");
    assert!(scoped.contains("(slot 0 x.0"), "{scoped}");
    assert!(scoped.contains("(slot 1 x.1"), "{scoped}");
}

/// §statements: the initializer is evaluated before the binding exists, so
/// `let x = x;` names the outer one.
/// §statements: a top-level binding is a global, so a `fn` can read one
/// without capturing anything — which is what keeps §functions' "a `fn`
/// carries no environment" true while the wall between the two comes down.
#[test]
fn a_top_level_binding_is_a_global() {
    let ir = ir("let n = 1u64; fn f() -> u64 { n } f()");
    assert!(ir.contains("(global @n u64)"), "{ir}");
    assert!(ir.contains("(globalget @n)"), "{ir}");
    // Not a capture, and so not a slot of `f`.
    assert!(!ir.contains("capture"), "{ir}");
}

/// Declarations hoist: a body is walked after the rest of its block, so the
/// order the two are written in does not matter.
#[test]
fn a_fn_reads_a_top_level_binding_declared_below_it() {
    assert!(
        lowered("fn f() -> u64 { n } let n = 1u64; f()")
            .1
            .is_empty()
    );
    assert!(
        lowered("let n = 1u64; fn f() -> u64 { n } f()")
            .1
            .is_empty()
    );
}

/// A binding inside a block is still a local, so §functions' rule still bites
/// there — a global is the top level's storage, not every block's.
#[test]
fn a_block_binding_is_still_a_local() {
    assert_eq!(
        only_error("fn outer() -> u64 { let a = 1u64; fn inner() -> u64 { a } inner() }"),
        "`a` is a local of an enclosing function, and a `fn` captures nothing — use a lambda"
    );
}

/// Initializers still run in order, so a call can reach a binding whose `let`
/// has not run. JavaScript answers this at run time; a static call graph lets
/// this one answer before the program starts.
#[test]
fn a_call_reaching_an_uninitialized_global_is_refused() {
    assert_eq!(
        only_error("let x = foo(); fn foo() -> u64 { y } let y = 1u64; x"),
        "`foo` reads `y`, which is not initialized until later in this file"
    );
    // Through one more call, which is why the reads are a fixed point.
    assert_eq!(
        only_error("let x = a(); fn a() -> u64 { b() } fn b() -> u64 { y } let y = 1u64; x"),
        "`a` reads `y`, which is not initialized until later in this file"
    );
    // The same call below the `let` it reads is fine.
    assert!(
        lowered("fn foo() -> u64 { y } let y = 1u64; foo()")
            .1
            .is_empty()
    );
}

#[test]
fn an_initializer_names_the_outer_binding() {
    let ir = ir("let x = 1u64; { let x = x; x }");
    assert!(ir.contains("(assign x (globalget @x))"), "{ir}");
}

/// §statements: assignment must match the binding's type. To give a name a
/// value of another type, declare it again.
#[test]
fn assignment_must_match_the_binding_type() {
    assert_eq!(
        only_error("let mut x = 1u64; x = \"no\"; x"),
        "expected `u64`, found `Str`"
    );
}

// ---- §control --------------------------------------------------------------

#[test]
fn a_jump_needs_a_loop() {
    assert_eq!(
        only_error("break;"),
        "`break` is only meaningful inside a loop"
    );
}

/// §control: a loop is a statement and its value is unit, so the `while`
/// carries no destination. §control: a jump written in a loop's *condition*
/// belongs to that loop — it is the innermost one enclosing it. The condition
/// needs a block expression to hold a statement, which is the only way to
/// write one there.
#[test]
fn a_jump_in_a_condition_belongs_to_its_loop() {
    let ir = ir("while { break; true } { }");
    assert!(ir.contains("(header 1\n        (break))"), "{ir}");
    // The body is the *second* block, and empty. Finding it by kind would find
    // the condition.
    assert!(ir.contains("(body 2)"), "{ir}");
}

#[test]
fn a_loop_has_no_value() {
    let ir = ir("let mut n = 0u64; while n < 3u64 { n = n + 1u64; } n");
    assert!(ir.contains("(cond t1)"), "{ir}");
}

// ---- §functions ------------------------------------------------------------

/// §functions: a nested `fn` is scoped to its block and captures nothing. To
/// capture, use a lambda.
#[test]
fn a_fn_captures_nothing() {
    assert_eq!(
        only_error("fn outer(n: u64) -> u64 { fn inner() -> u64 { n } inner() }"),
        "`n` is a local of an enclosing function, and a `fn` captures nothing — use a lambda"
    );
}

/// §functions: a lambda's types come from context. With none, that is an error
/// rather than a guess — and it is one error, not one per use.
#[test]
fn a_lambda_needs_context() {
    assert_eq!(
        only_error("fn apply(g: fn(u64) -> u64) -> u64 { g(1) } let f = (x) -> x; apply(f)"),
        "a lambda's types come from context, and there is none here — annotate its \
         parameters, or give the binding a type"
    );
}

/// The context can be an annotation on the binding, or the parameter the
/// lambda is being passed to.
#[test]
fn a_lambda_takes_its_types_from_context() {
    assert!(
        ir("let f: fn(u64) -> u64 = (x) -> x + 1; f(1)").contains("(func <file>.λ0 (u64) -> u64")
    );
    assert!(
        ir("fn apply(g: fn(u64) -> u64) -> u64 { g(1) } apply((x) -> x + 1)")
            .contains("(func <file>.λ0 (u64) -> u64")
    );
}

/// §functions: mutual recursion needs declarations to be order-independent, so
/// they are hoisted to the top of their block.
#[test]
fn declarations_are_hoisted() {
    let ir = ir("\
fn even(n: u64) -> bool { if n == 0 { true } else { odd(n - 1) } }
fn odd(n: u64) -> bool { if n == 0 { false } else { even(n - 1) } }
even(4)");
    assert!(ir.contains("(call odd"), "{ir}");
    assert!(ir.contains("(call even"), "{ir}");
}

/// §functions: arity and types are checked statically, so there is nothing
/// dynamic left to check at run time.
#[test]
fn arity_is_checked() {
    assert_eq!(
        only_error("fn f(a: u64) -> u64 { a } f(1, 2)"),
        "this function takes 1 argument(s), and 2 were given"
    );
}

/// §functions: a function is a value, and every function value is a closure —
/// a plain `fn` with an empty environment. Calling one is indirect.
#[test]
fn a_function_used_as_a_value_is_a_closure() {
    let ir = ir("fn add(a: u64, b: u64) -> u64 { a + b } let f = add; f(1, 2)");
    assert!(ir.contains("(closure add)"), "{ir}");
    assert!(
        ir.contains("(call-indirect t1 (const 1u64) (const 2u64))"),
        "{ir}"
    );
}

/// §functions: `mut` on a parameter is permission to mutate the argument in
/// place, and §statements is the rule that consumes it — the call site must
/// pass a `mut` binding, so a call that mutates is visible where it happens.
#[test]
fn a_mut_argument_needs_a_mut_binding() {
    assert_eq!(
        only_error(
            "struct C { n: u64 }
             fn advance(c: mut C) { c.n = c.n + 1; }
             let frozen = C { n: 0 };
             advance(frozen);"
        ),
        "the `c` parameter is `mut`, so `frozen` must be too — declare it `let mut frozen`"
    );
}

/// §functions: and the argument has to *be* a place, since there is nothing to
/// mutate in a value the call itself computed.
#[test]
fn a_mut_argument_must_be_a_place() {
    assert_eq!(
        only_error(
            "struct C { n: u64 }
             fn advance(c: mut C) { c.n = c.n + 1; }
             advance(C { n: 0 });"
        ),
        "the `c` parameter is `mut`, so the argument must be a binding rather than an expression"
    );
}

/// §functions' own example, end to end: a `mut` binding passed to a `mut`
/// parameter, mutated through a field. §types' reference semantics is what
/// carries the change back — nothing is written back at the call, and the IR
/// grows no node for it.
#[test]
fn a_mut_parameter_mutates_through_the_reference() {
    let ir = ir("struct C { n: u64 }
                 fn advance(c: mut C, by: u64) { c.n = c.n + by; }
                 let mut tally = C { n: 0 };
                 advance(tally, 5);");
    assert!(ir.contains("(slot 0 c  C param mut)"), "{ir}");
    assert!(ir.contains("(slot 1 by u64 param)"), "{ir}");
    assert!(ir.contains("(store (field c n) t3)"), "{ir}");
    assert!(ir.contains("(global @tally C mut)"), "{ir}");
    assert!(ir.contains("(drop (call advance t1 (const 5u64)))"), "{ir}");
}

/// §statements: a plain parameter is an immutable binding, exactly like a
/// `let`, so nothing may be mutated through it.
#[test]
fn a_plain_parameter_permits_no_mutation() {
    assert_eq!(
        only_error("struct C { n: u64 } fn f(c: C) { c.n = 1; }"),
        "`c` is not `mut`, so its fields cannot be assigned — write `let mut c`"
    );
}

/// §statements: `mut` gates in-place mutation and nothing else. Rebinding is
/// unrestricted on every binding, parameters included.
#[test]
fn assignment_needs_no_mut() {
    let ir = ir("fn f(n: u64) -> u64 { n = n + 1; n } let x = 1u64; f(x)");
    assert!(ir.contains("(slot 0 n u64 param)"), "{ir}");
}

// ---- §types Data and types -------------------------------------------------

/// §types: nominal. Two structs with identical fields are different types,
/// because identity comes from the act of construction.
#[test]
fn structs_are_nominal() {
    assert_eq!(
        only_error("struct A { x: u64 } struct B { x: u64 } fn f(a: A) -> A { a } f(B { x: 1 })"),
        "expected `A`, found `B`"
    );
}

/// §types: field mutability follows the binding. A non-`mut` binding permits
/// assigning no field.
#[test]
fn field_assignment_needs_a_mut_binding() {
    assert_eq!(
        only_error("struct P { x: u64 } let p = P { x: 1 }; p.x = 2;"),
        "`p` is not `mut`, so its fields cannot be assigned — write `let mut p`"
    );
}

/// §expressions: fields evaluate in written order; they are stored in
/// declaration order. The two are separated here rather than left for a back
/// end to guess.
#[test]
fn struct_fields_are_stored_in_declaration_order() {
    let ir = ir("struct P { x: u64, y: Str } let p = P { y: \"b\", x: 1 }; p");
    assert!(ir.contains("(struct (const 1u64) (const \"b\"))"), "{ir}");
}

#[test]
fn a_missing_field_is_named() {
    assert_eq!(
        only_error("struct P { x: u64, y: u64 } P { x: 1 }"),
        "field `y` is missing"
    );
}

/// §types: an alias is a second name for one type, not a new one.
#[test]
fn an_alias_is_the_same_type() {
    let ir = ir("type Id = u64; fn f(id: Id) -> u64 { id } f(1)");
    assert!(ir.contains("(func f (u64) -> u64"), "{ir}");
}

/// A struct may name itself, because §types gives structs reference semantics
/// and so a recursive one is ordinary rather than an infinite size.
#[test]
fn a_struct_may_name_itself() {
    let ir = ir("struct Node { next: Node, value: u64 } fn f(n: Node) -> u64 { n.value } 0u64");
    assert!(ir.contains("(field next Node)"), "{ir}");
}

// ---- The examples ----------------------------------------------------------

/// Every file in `examples/`, which `parser` already asserts lexes, parses,
/// and round-trips.
const EXAMPLES: [(&str, &str); 6] = [
    ("hello.glue", include_str!("../../examples/hello.glue")),
    (
        "literals.glue",
        include_str!("../../examples/literals.glue"),
    ),
    (
        "expressions.glue",
        include_str!("../../examples/expressions.glue"),
    ),
    (
        "statements.glue",
        include_str!("../../examples/statements.glue"),
    ),
    (
        "declarations.glue",
        include_str!("../../examples/declarations.glue"),
    ),
    ("tour.glue", include_str!("../../examples/tour.glue")),
];

/// The examples elaborate, and the only thing elaboration is allowed to say
/// about one is that the *language* is missing something.
///
/// They were written when nothing checked them, and they named functions and
/// types that had never existed. That is a worse kind of documentation than a
/// gap: `Unsupported` says "this construct is coming", where an unbound name
/// says "this is how Glue is written" and is wrong. So an example may exercise
/// syntax that has no meaning yet — the parser's suite needs it to — and may
/// not exercise a symbol that does not exist.
#[test]
fn the_examples_elaborate() {
    for (name, source) in EXAMPLES {
        let parse = parser::parse(source);
        let lowered = lower(&parse.tree, source);
        for diagnostic in &lowered.diagnostics {
            assert!(
                matches!(diagnostic.kind, DiagnosticKind::Unsupported(_)),
                "{name} does not elaborate: {}",
                diagnostic.message()
            );
        }
    }
}

// ---- Not yet ---------------------------------------------------------------

/// A construct the parser accepts and elaboration has no answer for says so,
/// which is a different thing to hear than a syntax error.
#[test]
fn unstarted_sections_say_so() {
    assert_eq!(
        only_error("let s = \"abc\"; s[0]"),
        "indexing is not supported yet"
    );
    assert_eq!(
        only_error("let s = \"abc\"; s.len()"),
        "method calls is not supported yet"
    );
}
