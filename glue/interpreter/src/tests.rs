//! Tests for execution.
//!
//! Written against source text rather than against a hand-built program: what
//! is worth pinning is what a program *means*, and that survived the move from
//! the concrete syntax tree to core IR — which is the move these tests were
//! written to survive.
//!
//! Two things about them read differently than they used to. First, most of
//! what this crate once refused at run time is refused before it runs, so
//! [`refused`] appears where [`trap`] used to. Second, §2 checks constant
//! expressions at compile time, so a test that wants a *trap* has to route its
//! operands through a function — `255u8 + 1` never runs at all.

use crate::error::TrapKind::{self, *};
use crate::value::{IntTy, Value};
use crate::{Error, run};

#[track_caller]
fn value(source: &str) -> Value {
    match run(source) {
        Ok(value) => value,
        Err(problem) => panic!("`{source}` should have run: {problem}"),
    }
}

#[track_caller]
fn int(source: &str) -> i128 {
    match value(source) {
        Value::Int { value, .. } => value,
        other => panic!("`{source}` should have been an integer, and was {other}"),
    }
}

#[track_caller]
fn boolean(source: &str) -> bool {
    match value(source) {
        Value::Bool(value) => value,
        other => panic!("`{source}` should have been a boolean, and was {other}"),
    }
}

/// The trap a program runs into.
#[track_caller]
fn trap(source: &str) -> TrapKind {
    match run(source) {
        Err(Error::Runtime(error)) => error.kind,
        Ok(value) => panic!("`{source}` should have trapped, and was {value}"),
        Err(problem) => panic!("`{source}` should have trapped: {problem}"),
    }
}

/// The one elaboration diagnostic a program is refused for.
///
/// The diagnostics themselves are `ir`'s to test; what is worth checking here
/// is that a program carrying one does not run.
#[track_caller]
fn refused(source: &str) -> String {
    match run(source) {
        Err(Error::Elaboration(diagnostics)) => {
            assert_eq!(
                diagnostics.len(),
                1,
                "`{source}` should have had one diagnostic: {diagnostics:#?}"
            );
            diagnostics[0].message()
        }
        Ok(value) => panic!("`{source}` should have been refused, and was {value}"),
        Err(problem) => panic!("`{source}` should have been refused by elaboration: {problem}"),
    }
}

// ---- A program is a block --------------------------------------------------

/// Goal §2.1: a bare expression is a valid program. Not a REPL special case —
/// the block rule (§3) applied to the outermost block, and by now not even
/// that: elaboration makes the file an ordinary function.
#[test]
fn a_file_is_worth_its_trailing_expression() {
    assert_eq!(int("42"), 42);
    assert_eq!(int("let x = 2; x * 21"), 42);
}

#[test]
fn a_file_ending_in_a_statement_is_worth_unit() {
    assert_eq!(value("let x = 42;"), Value::Unit);
}

#[test]
fn an_empty_file_is_worth_unit() {
    assert_eq!(value(""), Value::Unit);
    assert_eq!(value("// nothing but a comment\n"), Value::Unit);
}

// ---- Literals and their types (§1) -----------------------------------------

#[test]
fn literals_decode() {
    assert_eq!(value("42"), Value::u64(42));
    assert_eq!(value("1_000_000"), Value::u64(1_000_000));
    assert_eq!(value("0xff"), Value::u64(255));
    assert_eq!(value("0o17"), Value::u64(15));
    assert_eq!(value("0b1010"), Value::u64(10));
    assert_eq!(value("1.5"), Value::f64(1.5));
    assert_eq!(value(".5"), Value::f64(0.5));
    assert_eq!(value("1e3"), Value::f64(1000.0));
    assert_eq!(value("true"), Value::Bool(true));
    assert_eq!(value("'x'"), Value::Char('x'));
    assert_eq!(value(r#""hi\n""#), Value::string("hi\n"));
    assert_eq!(value("()"), Value::Unit);
}

/// §1: an unpinned constant takes its type from context; with none, the sign
/// decides. This is the whole of what "there are types now" changed about a
/// literal.
#[test]
fn an_unpinned_constant_pins_by_sign() {
    assert_eq!(value("1"), Value::u64(1));
    assert_eq!(value("-1"), Value::s64(-1));
    assert_eq!(
        value("let x: u8 = 200; x"),
        Value::int(200, IntTy::new(false, 8))
    );
}

#[test]
fn a_suffix_pins_a_width() {
    assert_eq!(value("255u8"), Value::int(255, IntTy::new(false, 8)));
    assert_eq!(value("1f32"), Value::f32(1.0));
}

/// The annotation used to be read and dropped, because there was nothing to
/// check it against. §10 arrived.
#[test]
fn an_annotation_is_checked() {
    assert_eq!(
        refused("let x: u8 = 300; x"),
        "the constant 300 does not fit in `u8`"
    );
}

// ---- Arithmetic (§2) -------------------------------------------------------

/// Through a function, because §2 folds and checks a constant expression at
/// compile time — so `2 + 3` never reaches the executor, and a test of the
/// executor has to hand it something that isn't constant.
#[test]
fn arithmetic_evaluates() {
    assert_eq!(int("fn f(a: u64, b: u64) -> u64 { a + b } f(2, 3)"), 5);
    assert_eq!(int("fn f(a: u64, b: u64) -> u64 { a - b } f(5, 3)"), 2);
    assert_eq!(int("fn f(a: u64, b: u64) -> u64 { a * b } f(6, 7)"), 42);
    assert_eq!(int("fn f(a: u64, b: u64) -> u64 { a / b } f(7, 2)"), 3);
    assert_eq!(int("fn f(a: u64, b: u64) -> u64 { a % b } f(7, 2)"), 1);
    assert_eq!(
        value("fn f(a: f64, b: f64) -> f64 { a * b } f(1.5, 2.0)"),
        Value::f64(3.0)
    );
}

/// §2: division truncates toward zero and the remainder takes the dividend's
/// sign — `div_s` and `rem_s`, which is what wasm does natively.
#[test]
fn division_truncates_and_remainder_follows_the_dividend() {
    assert_eq!(int("fn f(a: s64, b: s64) -> s64 { a / b } f(-7, 2)"), -3);
    assert_eq!(int("fn f(a: s64, b: s64) -> s64 { a % b } f(-7, 2)"), -1);
    assert_eq!(int("fn f(a: s64, b: s64) -> s64 { a % b } f(7, -2)"), 1);
}

/// §2: overflow is an error rather than a wrap — and §1's widths are what make
/// the same addition an error at one type and an answer at another. The old
/// interpreter, where every integer was an `i64`, could not tell these apart.
#[test]
fn overflow_traps_at_the_operands_width() {
    assert_eq!(
        trap("fn f(a: u8, b: u8) -> u8 { a + b } f(255, 1)"),
        Overflow {
            operator: "+",
            ty: "u8".to_string(),
        }
    );
    assert_eq!(int("fn f(a: u16, b: u16) -> u16 { a + b } f(255, 1)"), 256);
    assert_eq!(
        trap("fn f(a: u64, b: u64) -> u64 { a - b } f(0, 1)"),
        Overflow {
            operator: "-",
            ty: "u64".to_string(),
        }
    );
}

#[test]
fn division_by_zero_traps() {
    assert_eq!(
        trap("fn f(a: u64, b: u64) -> u64 { a / b } f(1, 0)"),
        DividedByZero
    );
    assert_eq!(
        trap("fn f(a: u64, b: u64) -> u64 { a % b } f(1, 0)"),
        DividedByZero
    );
}

/// §2: floats follow IEEE-754, and IEEE's answer to division by zero is an
/// infinity. Trapping is for the operation with no representable answer.
#[test]
fn float_division_by_zero_follows_ieee() {
    assert_eq!(
        value("fn f(a: f64, b: f64) -> f64 { a / b } f(1.0, 0.0)"),
        Value::f64(f64::INFINITY)
    );
}

/// §2: "constant expressions are checked at compile time rather than
/// trapping". The trap and the diagnostic are the same rule at two stages, and
/// this is the stage that moved.
#[test]
fn a_constant_that_would_trap_is_refused_instead() {
    assert_eq!(
        refused("255u8 + 1"),
        "this constant expression overflows — constants are checked at compile time"
    );
    assert_eq!(refused("1 / 0"), "this constant expression divides by zero");
}

#[test]
fn unary_operators_apply() {
    assert_eq!(int("fn f(a: s64) -> s64 { -a } f(1)"), -1);
    assert!(!boolean("fn f(a: bool) -> bool { !a } f(true)"));
    assert_eq!(value("fn f(a: f64) -> f64 { -a } f(1.5)"), Value::f64(-1.5));
}

// ---- Comparison and logic (§2) ---------------------------------------------

#[test]
fn comparison_evaluates() {
    assert!(boolean("fn f(a: u64, b: u64) -> bool { a < b } f(1, 2)"));
    assert!(!boolean("fn f(a: u64, b: u64) -> bool { a > b } f(1, 2)"));
    assert!(boolean("fn f(a: u64, b: u64) -> bool { a <= b } f(2, 2)"));
    assert!(boolean("fn f(a: u64, b: u64) -> bool { a == b } f(2, 2)"));
    assert!(boolean("fn f(a: u64, b: u64) -> bool { a != b } f(2, 3)"));
    assert!(boolean(
        "fn f(a: char, b: char) -> bool { a < b } f('a', 'b')"
    ));
}

/// §2: IEEE-754, so every ordering against NaN is false and `NaN != NaN`.
#[test]
fn nan_compares_unequal_to_everything() {
    let nan = "let nan = 0.0 / 0.0;";
    assert!(!boolean(&format!(
        "{nan} fn f(x: f64) -> bool {{ x == x }} f(nan)"
    )));
    assert!(boolean(&format!(
        "{nan} fn f(x: f64) -> bool {{ x != x }} f(nan)"
    )));
    assert!(!boolean(&format!(
        "{nan} fn f(x: f64) -> bool {{ x < 1.0 }} f(nan)"
    )));
    assert!(!boolean(&format!(
        "{nan} fn f(x: f64) -> bool {{ x >= 1.0 }} f(nan)"
    )));
}

/// §2: `&&` and `||` short-circuit, which makes them control flow wearing an
/// operator's clothes — so the right operand's trap never happens. Elaboration
/// lowers them to a branch, and there is no lazy operator in the IR for a back
/// end to get wrong.
#[test]
fn logical_operators_short_circuit() {
    let and = "fn f(a: bool, n: u64) -> bool { a && 10 / n == 0 }";
    assert!(!boolean(&format!("{and} f(false, 0)")));
    assert_eq!(trap(&format!("{and} f(true, 0)")), DividedByZero);

    let or = "fn f(a: bool, n: u64) -> bool { a || 10 / n == 0 }";
    assert!(boolean(&format!("{or} f(true, 0)")));
    assert_eq!(trap(&format!("{or} f(false, 0)")), DividedByZero);
}

// ---- Strings ---------------------------------------------------------------

#[test]
fn strings_concatenate_with_plus() {
    assert_eq!(
        value(r#"fn f(a: Str, b: Str) -> Str { a + b } f("a", "bc")"#),
        Value::string("abc")
    );
}

/// §1: strings are UTF-8, so byte order is code-point order.
#[test]
fn strings_compare_by_bytes() {
    assert!(boolean(
        r#"fn f(a: Str, b: Str) -> bool { a < b } f("apple", "banana")"#
    ));
    assert!(boolean(
        r#"fn f(a: Str, b: Str) -> bool { a == b } f("a", "a")"#
    ));
}

// ---- Bindings (§3) ---------------------------------------------------------

#[test]
fn a_binding_holds_its_value() {
    assert_eq!(int("let x = 41; x + 1"), 42);
}

/// §3: `mut` gates in-place mutation only. Rebinding is unrestricted on every
/// binding, so this needs no `mut` — which is the opposite of what this crate
/// used to enforce.
#[test]
fn assignment_needs_no_mut() {
    assert_eq!(int("let x = 1u64; x = 2; x"), 2);
}

/// §3: shadowing is allowed, including in the same scope — the natural way to
/// write a narrowing pipeline without inventing `input2`.
#[test]
fn a_let_may_shadow_in_the_same_scope() {
    assert_eq!(int("let x = 1; let x = x + 1; x"), 2);
}

/// §1: a binding whose initializer is a constant expression and which is never
/// assigned stays unpinned, so this is `-2` rather than an underflow of `u64`.
#[test]
fn an_unassigned_constant_binding_stays_unpinned() {
    assert_eq!(value("let n = 3; n - 5"), Value::s64(-2));
}

// ---- Blocks (§2) -----------------------------------------------------------

#[test]
fn a_block_is_worth_its_trailing_expression() {
    assert_eq!(int("let x = { let y = 2; y * 21 }; x"), 42);
}

#[test]
fn a_semicolon_discards_a_blocks_value() {
    assert_eq!(value("let x = { 42; }; x"), Value::Unit);
}

#[test]
fn a_block_scopes_its_bindings() {
    assert_eq!(int("let x = 1; { let x = 2; } x"), 1);
    assert_eq!(
        refused("{ let inner = 1; } inner"),
        "no binding named `inner` is in scope"
    );
}

// ---- Control flow (§4) -----------------------------------------------------

#[test]
fn if_is_an_expression() {
    assert_eq!(int("let x = if true { 1 } else { 2 }; x"), 1);
    assert_eq!(
        int("fn f(n: u64) -> u64 { if n < 10 { 1 } else if n < 20 { 2 } else { 3 } } f(15)"),
        2
    );
}

/// §2: with no `else` its value is unit, which is what makes it usable as a
/// statement and not as a value.
#[test]
fn an_if_with_no_else_is_worth_unit() {
    assert_eq!(value("if false { 1 }"), Value::Unit);
}

#[test]
fn while_loops() {
    assert_eq!(
        int("let mut i = 0u64;
             let mut total = 0u64;
             while i < 5 {
               total = total + i;
               i = i + 1;
             }
             total"),
        10
    );
}

/// §4: unlabelled, and applying to the innermost enclosing loop.
#[test]
fn break_and_continue_apply_to_the_nearest_loop() {
    assert_eq!(
        int("let mut i = 0u64;
             while true {
               i = i + 1;
               if i > 3 { break; }
             }
             i"),
        4
    );
    assert_eq!(
        int("let mut i = 0u64;
             let mut odd = 0u64;
             while i < 6 {
               i = i + 1;
               if i % 2 == 0 { continue; }
               odd = odd + 1;
             }
             odd"),
        3
    );
}

/// §4: the header runs every iteration, because a condition is re-evaluated
/// every iteration — so a condition with an effect has it every time round.
#[test]
fn a_condition_runs_every_iteration() {
    assert_eq!(int("let mut i = 0u64; while { i = i + 1; i < 3 } { } i"), 3);
}

/// And a jump written in the condition belongs to the loop it conditions,
/// which is the same reading elaboration takes when it checks that a jump has
/// a loop at all.
#[test]
fn a_jump_in_a_condition_leaves_its_loop() {
    assert_eq!(
        int("let mut i = 0u64;
             while { i = i + 1; if i > 2 { break; } true } { }
             i"),
        3
    );
}

/// §4: a jump needs a loop, and elaboration is where that is settled now.
#[test]
fn a_jump_outside_a_loop_is_refused() {
    assert_eq!(
        refused("break;"),
        "`break` is only meaningful inside a loop"
    );
}

// ---- Functions (§5) --------------------------------------------------------

#[test]
fn a_function_is_declared_and_called() {
    assert_eq!(int("fn double(n: u64) -> u64 { n * 2 } double(21)"), 42);
}

/// §5: a body is a block, so its value is its trailing expression; `return` is
/// the early exit a well-shaped function often has none of.
#[test]
fn a_body_is_a_block_and_return_is_the_early_exit() {
    assert_eq!(
        int("fn clamp(n: u64) -> u64 { if n > 10 { return 10; } n } clamp(42)"),
        10
    );
    assert_eq!(
        int("fn clamp(n: u64) -> u64 { if n > 10 { return 10; } n } clamp(3)"),
        3
    );
}

/// §5: unit is a real value with one inhabitant, not an absence.
#[test]
fn a_function_with_no_return_type_returns_unit() {
    assert_eq!(value("fn nothing() { } nothing()"), Value::Unit);
}

/// §5: mutual recursion needs declarations to be order-independent, which is
/// what hoisting them per block buys.
#[test]
fn functions_recurse_and_see_each_other() {
    assert_eq!(
        int("fn factorial(n: u64) -> u64 {
               if n == 0 { 1 } else { n * factorial(n - 1) }
             }
             factorial(10)"),
        3628800
    );
    assert!(boolean(
        "fn even(n: u64) -> bool { if n == 0 { true } else { odd(n - 1) } }
         fn odd(n: u64) -> bool { if n == 0 { false } else { even(n - 1) } }
         even(10)"
    ));
}

/// §5: there is no tail-call guarantee, so an unbounded recursion traps —
/// raised at a depth the host stack still has room for rather than by falling
/// off it.
#[test]
fn runaway_recursion_traps() {
    assert_eq!(
        trap("fn forever(n: u64) -> u64 { forever(n) } forever(1)"),
        RecursionLimit
    );
}

/// §5 checks arity statically, and now something does.
#[test]
fn arity_is_checked_before_it_runs() {
    assert_eq!(
        refused("fn f(a: u64) -> u64 { a } f(1, 2)"),
        "this function takes 1 argument(s), and 2 were given"
    );
}

// ---- Functions as values, and lambdas (§5) ---------------------------------

/// §5: a function is a value. Every function value is a closure, and a plain
/// `fn` is one with an empty environment — so a name and a lambda are called
/// the same way, indirectly.
#[test]
fn a_function_is_a_value() {
    assert_eq!(
        int("fn add(a: u64, b: u64) -> u64 { a + b } let f = add; f(1, 2)"),
        3
    );
    assert_eq!(
        int("fn twice(g: fn(u64) -> u64, x: u64) -> u64 { g(g(x)) }
             fn increment(n: u64) -> u64 { n + 1 }
             twice(increment, 40)"),
        42
    );
}

#[test]
fn a_lambda_is_a_value_and_calls() {
    assert_eq!(
        int("fn apply(g: fn(u64) -> u64, x: u64) -> u64 { g(x) } apply((n) -> n * 2, 21)"),
        42
    );
    assert_eq!(
        int("let double: fn(u64) -> u64 = (n) -> n * 2; double(21)"),
        42
    );
}

/// §5: a lambda captures by reference, so mutation through one holder is
/// visible to every other. A binding that is captured *and* assigned is a cell,
/// which is where that sharing lives.
#[test]
fn a_lambda_captures_by_reference() {
    assert_eq!(
        int("let mut n = 0u64;
             let bump = () -> { n = n + 1; n };
             let read = () -> n;
             bump();
             bump();
             read()"),
        2
    );
}

/// §5: a captured binding outlives the frame that created it. The cell is
/// heap-allocated and the closure holds it, so returning the lambda keeps the
/// binding alive after `counter` has returned.
#[test]
fn a_capture_outlives_its_frame() {
    assert_eq!(
        int("fn counter() -> fn() -> u64 {
               let mut n = 0;
               () -> { n = n + 1; n }
             }
             let c = counter();
             c();
             c();
             c()"),
        3
    );
}

/// §5's per-iteration promise: a `let` in a loop body is a fresh binding each
/// time round, captured separately — the classic loop-variable trap, absent by
/// construction. `snapshot` is never assigned, so each iteration's value is
/// copied into the closure rather than shared through a cell, and the lambda
/// made on the second iteration still says `1`.
#[test]
fn a_binding_in_a_loop_is_captured_per_iteration() {
    assert_eq!(
        int("let mut i = 0u64;
             let mut f: fn() -> u64 = () -> 0;
             while i < 3 {
               let snapshot = i;
               if i == 1 { f = () -> snapshot; }
               i = i + 1;
             }
             f()"),
        1
    );
}

/// §5: a `fn` captures nothing — it is an ordinary function that happens to be
/// private to its block, and every one of them compiles to a plain wasm
/// function with no environment.
#[test]
fn a_fn_captures_nothing() {
    assert_eq!(
        refused("fn outer(n: u64) -> u64 { fn inner() -> u64 { n } inner() }"),
        "`n` is a local of an enclosing function, and a `fn` captures nothing — use a lambda"
    );
}

/// §5: a lambda's types come from context, unlike a named `fn` — a `fn` is a
/// declaration others read, a lambda is an argument read in place.
#[test]
fn a_lambda_needs_context() {
    assert_eq!(
        refused("let f = (x) -> x + 1; f(1)"),
        "a lambda's types come from context, and there is none here — annotate its parameters, \
         or give the binding a type"
    );
}

/// §2 defines equality on values and identity on instance references, and says
/// nothing about functions.
#[test]
fn functions_do_not_compare() {
    assert_eq!(
        refused("fn f() { } fn g() { } let a = f; let b = g; a == b"),
        "`==` is not defined on `(fn ())`"
    );
}

// ---- What is refused before it runs ----------------------------------------

/// A half-parsed tree is what the language server wants and what an interpreter
/// does not: evaluating around an error node means guessing at the missing text.
#[test]
fn a_program_that_did_not_parse_does_not_run() {
    let Err(Error::Syntax(errors)) = run("let = 5;") else {
        panic!("a program that did not parse should not have run");
    };
    assert!(!errors.is_empty());
}

#[test]
fn a_lexical_error_is_reported_even_when_the_parse_survives() {
    let Err(Error::Syntax(errors)) = run(r#""\q""#) else {
        panic!("a program with a bad escape should not have run");
    };
    assert_eq!(errors.len(), 1);
}

/// The same rule one stage later: elaboration is total, so a program with an
/// unbound name still produces IR. Running it would mean guessing.
#[test]
fn a_program_that_did_not_elaborate_does_not_run() {
    assert_eq!(refused("x"), "no binding named `x` is in scope");
    assert_eq!(
        refused("fn f(n: u64) -> u64 { if n { 1 } else { 2 } } f(1)"),
        "a condition must be `bool`, and this is `u64` — there is no truthiness"
    );
    assert_eq!(
        refused("fn f(a: u64, b: f64) -> u64 { a + b } f(1, 1.5)"),
        "`+` needs both operands to have the same type, and these are `u64` and `f64` — \
         conversions are explicit"
    );
}

// ---- The edges of this stage -----------------------------------------------

/// IR this executor does not run yet. Each one elaborates cleanly — the gap is
/// here, not in front of it — and each is scheduled.
#[test]
fn unrun_constructs_say_so() {
    assert_eq!(
        trap("struct P { x: u64 } let p = P { x: 1 }; 0u64"),
        Unsupported("a struct literal")
    );
    assert_eq!(
        trap("fn f(n: u64) -> u8 { n as u8 } f(1)"),
        Unsupported("the `as` operator")
    );
}

// ---- Echoing a value -------------------------------------------------------

#[test]
fn values_display_as_they_are_echoed() {
    assert_eq!(value("42").to_string(), "42");
    assert_eq!(value("-1").to_string(), "-1");
    assert_eq!(value("1.5").to_string(), "1.5");
    assert_eq!(value("2.0").to_string(), "2.0");
    assert_eq!(value("true").to_string(), "true");
    assert_eq!(value("'x'").to_string(), "'x'");
    assert_eq!(value(r#""hi""#).to_string(), "\"hi\"");
    assert_eq!(value("()").to_string(), "()");
    // A function value shows the name elaboration gave it, which for a lambda
    // is its parent's plus `.λn`.
    assert_eq!(
        value("fn add(a: u64, b: u64) -> u64 { a + b } let f = add; f").to_string(),
        "<fn add>"
    );
    assert_eq!(
        value("let f: fn(u64) -> u64 = (n) -> n; f").to_string(),
        "<fn <file>.\u{3bb}0>"
    );
}
