//! Tests for evaluation.
//!
//! Written against the source text rather than against a hand-built tree: the
//! thing worth pinning is what a program means, and that has to survive the
//! move to the IL, where a hand-built concrete tree would not.

use crate::error::RuntimeErrorKind::{self, *};
use crate::value::Value;
use crate::{Error, run};

#[track_caller]
fn value(source: &str) -> Value {
    match run(source) {
        Ok(value) => value,
        Err(problem) => panic!("`{source}` should have run: {problem}"),
    }
}

#[track_caller]
fn int(source: &str) -> i64 {
    match value(source) {
        Value::Int(value) => value,
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

#[track_caller]
fn failure(source: &str) -> RuntimeErrorKind {
    match run(source) {
        Err(Error::Runtime(error)) => error.kind,
        Ok(value) => panic!("`{source}` should have failed, and was {value}"),
        Err(problem) => panic!("`{source}` should have failed at runtime: {problem}"),
    }
}

// ---- A program is a block --------------------------------------------------

/// Goal §2.1: a bare expression is a valid program. Not a REPL special case —
/// the block rule (§3) applied to the outermost block.
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

// ---- Literals --------------------------------------------------------------

#[test]
fn literals_decode() {
    assert_eq!(value("42"), Value::Int(42));
    assert_eq!(value("1_000_000"), Value::Int(1_000_000));
    assert_eq!(value("0xff"), Value::Int(255));
    assert_eq!(value("0o17"), Value::Int(15));
    assert_eq!(value("0b1010"), Value::Int(10));
    assert_eq!(value("1.5"), Value::Float(1.5));
    assert_eq!(value(".5"), Value::Float(0.5));
    assert_eq!(value("1e3"), Value::Float(1000.0));
    assert_eq!(value("true"), Value::Bool(true));
    assert_eq!(value("'x'"), Value::Char('x'));
    assert_eq!(value(r#""hi\n""#), Value::string("hi\n"));
    assert_eq!(value("()"), Value::Unit);
}

/// §1 makes an unsuffixed literal an unpinned constant, and pinning is §10's.
/// So a suffix decides one thing here: integer or float.
#[test]
fn a_float_suffix_makes_an_integer_literal_a_float() {
    assert_eq!(value("1f64"), Value::Float(1.0));
    assert_eq!(value("1_000u32"), Value::Int(1000));
    assert_eq!(value("7s16"), Value::Int(7));
}

/// Not a language rule — see the note on `Value`. Pinned so that the day §10
/// gives integers real widths, this test is the one that has to change.
#[test]
fn an_integer_wider_than_i64_is_refused() {
    assert_eq!(failure("18446744073709551615"), IntegerTooLarge);
}

// ---- Arithmetic (§2) -------------------------------------------------------

#[test]
fn arithmetic_evaluates() {
    assert_eq!(int("1 + 2"), 3);
    assert_eq!(int("3 - 4"), -1);
    assert_eq!(int("5 * 6"), 30);
    assert_eq!(int("9 % 10"), 9);
    assert_eq!(value("1.0 + 2.5"), Value::Float(3.5));
}

/// §2: integer division truncates toward zero, matching wasm's `div_s`, and
/// remainder takes the sign of the dividend, matching `rem_s`.
#[test]
fn division_truncates_and_remainder_follows_the_dividend() {
    assert_eq!(int("7 / 2"), 3);
    assert_eq!(int("-7 / 2"), -3);
    assert_eq!(int("-7 % 2"), -1);
    assert_eq!(int("7 % -2"), 1);
}

/// §2: overflow is an error, not a wrap. wasm's native behaviour is silent
/// wrapping, so this is a check paid for deliberately.
#[test]
fn overflow_traps() {
    assert_eq!(failure("9223372036854775807 + 1"), Overflow("+"));
    assert_eq!(failure("9223372036854775807 * 2"), Overflow("*"));
}

#[test]
fn division_by_zero_traps() {
    assert_eq!(failure("1 / 0"), DividedByZero);
    assert_eq!(failure("1 % 0"), DividedByZero);
}

/// §2 also says floats follow IEEE-754, and IEEE's answer to division by zero
/// is an infinity rather than an absence of one.
#[test]
fn float_division_by_zero_follows_ieee() {
    assert_eq!(value("1.0 / 0.0"), Value::Float(f64::INFINITY));
}

/// §1 has no implicit conversion, so there is no widening to reach for.
#[test]
fn mixing_an_integer_and_a_float_is_an_error() {
    assert_eq!(
        failure("1 + 1.5"),
        BinaryTypeMismatch {
            operator: "+",
            left: "an integer",
            right: "a float",
        }
    );
}

#[test]
fn precedence_follows_the_ladder() {
    assert_eq!(int("1 + 2 * 3"), 7);
    assert_eq!(int("(1 + 2) * 3"), 9);
    assert_eq!(int("-2 * 3"), -6);
    assert_eq!(
        int("10 - 3 - 2"),
        5,
        "every binary operator is left-associative"
    );
    assert!(boolean("1 + 1 == 2"));
    assert!(boolean("true || false && false"));
}

#[test]
fn unary_operators_apply() {
    assert_eq!(int("-1"), -1);
    assert_eq!(int("--1"), 1);
    assert!(!boolean("!true"));
    assert_eq!(
        failure("!1"),
        UnaryTypeMismatch {
            operator: "!",
            operand: "an integer",
        }
    );
}

// ---- Comparison and logic (§2) ---------------------------------------------

#[test]
fn comparison_evaluates() {
    assert!(boolean("1 == 1"));
    assert!(boolean("1 != 2"));
    assert!(boolean("1 < 2"));
    assert!(boolean("2 >= 2"));
    assert!(boolean("'a' < 'b'"));
    assert!(boolean(r#""abc" < "abd""#), "strings compare by bytes");
    assert!(boolean("() == ()"));
}

/// §2: `1 == "1"` is a type error, not `false`.
#[test]
fn cross_type_comparison_does_not_exist() {
    assert_eq!(
        failure(r#"1 == "1""#),
        BinaryTypeMismatch {
            operator: "==",
            left: "an integer",
            right: "a string",
        }
    );
}

/// Equatable but not ordered: §2 gives no meaning to `false < true`.
#[test]
fn booleans_are_not_ordered() {
    assert!(boolean("true == true"));
    assert_eq!(
        failure("true < false"),
        BinaryTypeMismatch {
            operator: "<",
            left: "a boolean",
            right: "a boolean",
        }
    );
}

/// §2: floats follow IEEE-754, so `NaN != NaN` and all four orderings against
/// it are false.
#[test]
fn nan_compares_unequal_to_everything() {
    let nan = "let nan = 0.0 / 0.0;";
    assert!(!boolean(&format!("{nan} nan == nan")));
    assert!(boolean(&format!("{nan} nan != nan")));
    assert!(!boolean(&format!("{nan} nan < 1.0")));
    assert!(!boolean(&format!("{nan} nan >= 1.0")));
}

/// §2: `&&` and `||` short-circuit, which makes them control flow wearing an
/// operator's clothes — so the right operand's error never happens.
#[test]
fn logical_operators_short_circuit() {
    assert!(!boolean("false && 1 / 0 == 0"));
    assert!(boolean("true || 1 / 0 == 0"));
    assert_eq!(failure("true && 1 / 0 == 0"), DividedByZero);
}

#[test]
fn logical_operators_take_booleans_only() {
    assert_eq!(
        failure("1 && true"),
        UnaryTypeMismatch {
            operator: "&&",
            operand: "an integer",
        }
    );
}

// ---- Strings ---------------------------------------------------------------

#[test]
fn strings_concatenate_with_plus() {
    assert_eq!(value(r#""a" + "bc""#), Value::string("abc"));
    assert_eq!(
        failure(r#""a" - "b""#),
        BinaryTypeMismatch {
            operator: "-",
            left: "a string",
            right: "a string",
        }
    );
}

// ---- Bindings (§3) ---------------------------------------------------------

#[test]
fn a_binding_holds_its_value() {
    assert_eq!(int("let x = 41; x + 1"), 42);
    assert_eq!(failure("x"), UnknownName("x".to_string()));
}

/// §3: `mut` gates mutation, not rebinding.
#[test]
fn assignment_needs_mut() {
    assert_eq!(int("let mut x = 1; x = 2; x"), 2);
    assert_eq!(
        failure("let x = 1; x = 2; x"),
        ImmutableBinding("x".to_string())
    );
    assert_eq!(failure("x = 1;"), UnknownName("x".to_string()));
}

/// §3: shadowing is allowed, including in the same scope — the natural way to
/// write a narrowing pipeline without inventing `input2`.
#[test]
fn a_let_may_shadow_in_the_same_scope() {
    assert_eq!(int("let x = 1; let x = x + 1; x"), 2);
}

/// §3's place is a name, a field, or an index. The other two need §6's types,
/// and the check lives here so the message can name what was assigned to.
#[test]
fn only_a_place_can_be_assigned_to() {
    assert_eq!(failure("let mut x = 1; x + 1 = 2;"), NotAPlace);
}

/// Read and ignored — there is nothing to check it against yet. This test is a
/// marker for §10, not an endorsement.
#[test]
fn a_type_annotation_is_ignored() {
    assert_eq!(int("let x: u8 = 300; x"), 300);
}

// ---- Blocks (§2) -----------------------------------------------------------

#[test]
fn a_block_is_worth_its_trailing_expression() {
    assert_eq!(int("{ let t = 21; t * 2 }"), 42);
}

/// §2's semicolon rule: a `;` discards.
#[test]
fn a_semicolon_discards_a_blocks_value() {
    assert_eq!(value("{ 42; }"), Value::Unit);
    assert_eq!(value("{ let x = 1; }"), Value::Unit);
}

#[test]
fn a_block_scopes_its_bindings() {
    assert_eq!(int("let x = 1; { let x = 2; }; x"), 1);
    assert_eq!(
        failure("{ let inner = 1; }; inner"),
        UnknownName("inner".to_string())
    );
    assert_eq!(
        int("let mut x = 1; { x = 2; }; x"),
        2,
        "assignment reaches out"
    );
}

// ---- Control flow (§4) -----------------------------------------------------

#[test]
fn if_is_an_expression() {
    assert_eq!(int("if true { 1 } else { 2 }"), 1);
    assert_eq!(int("if false { 1 } else { 2 }"), 2);
    assert_eq!(int("if false { 1 } else if true { 2 } else { 3 }"), 2);
}

/// §2: an `if` with no `else` has type unit, so it is usable as a statement but
/// not as a value.
#[test]
fn an_if_with_no_else_is_worth_unit() {
    assert_eq!(value("if false { 1 }"), Value::Unit);
}

/// §2: there is no truthiness. This falls out of §1 having no `nil` and no
/// implicit conversion.
#[test]
fn a_condition_must_be_a_boolean() {
    assert_eq!(
        failure("if 1 { 2 } else { 3 }"),
        ConditionNotBool("an integer")
    );
    assert_eq!(failure("while 1 { }"), ConditionNotBool("an integer"));
}

#[test]
fn while_loops() {
    assert_eq!(
        int("let mut total = 0; while total < 10 { total = total + 1; } total"),
        10
    );
}

/// §4: unlabelled, applying to the innermost enclosing loop.
#[test]
fn break_and_continue_apply_to_the_nearest_loop() {
    assert_eq!(
        int("let mut n = 0; while true { n = n + 1; if n > 3 { break; } } n"),
        4
    );
    assert_eq!(
        int(
            "let mut n = 0; let mut seen = 0; while n < 5 { n = n + 1; if n < 3 { continue; } seen = seen + 1; } seen"
        ),
        3
    );
}

#[test]
fn break_outside_a_loop_is_an_error() {
    assert_eq!(failure("break;"), BreakOutsideLoop);
    assert_eq!(failure("continue;"), ContinueOutsideLoop);
}

/// §5: a binding made in a loop body is fresh each iteration, which is why the
/// classic loop-variable capture trap is absent.
#[test]
fn a_loop_body_is_a_fresh_scope() {
    assert_eq!(
        int("let mut n = 0; while n < 3 { let step = 1; n = n + step; } n"),
        3
    );
}

// ---- The edges of this stage -----------------------------------------------

#[test]
fn unimplemented_constructs_say_so() {
    assert_eq!(
        failure("struct P { x: u64, }"),
        Unsupported("a struct declaration")
    );
    assert_eq!(failure("type T = u64;"), Unsupported("a type alias"));
    assert_eq!(failure("1 as u8"), Unsupported("the `as` operator"));
    assert_eq!(failure("let a = 1; a[0]"), Unsupported("indexing"));
    assert_eq!(failure("let a = 1; a.x"), Unsupported("field access"));
    assert_eq!(failure("let a = 1; a.next()"), Unsupported("a method call"));
}

// ---- Functions (§5) --------------------------------------------------------

#[test]
fn a_function_is_declared_and_called() {
    assert_eq!(int("fn add(a: u64, b: u64) -> u64 { a + b } add(1, 2)"), 3);
    assert_eq!(int("fn one() -> u64 { 1 } one()"), 1);
}

/// §5: the body is a block, so its value is its trailing expression, and
/// `return` is for *early* exit — a well-shaped function often has none.
#[test]
fn a_body_is_a_block_and_return_is_the_early_exit() {
    let clamp = "fn clamp(n: u64, limit: u64) -> u64 { if n > limit { return limit; } n }";
    assert_eq!(int(&format!("{clamp} clamp(3, 10)")), 3);
    assert_eq!(int(&format!("{clamp} clamp(30, 10)")), 10);
}

/// §5: a function with no `-> T` returns unit, which is a real value rather
/// than an absence — it can be bound, returned, and stored.
#[test]
fn a_function_with_no_return_type_returns_unit() {
    assert_eq!(value("fn nothing() { } nothing()"), Value::Unit);
    assert_eq!(value("fn nothing() { return; } nothing()"), Value::Unit);
    assert_eq!(value("fn nothing() { } let x = nothing(); x"), Value::Unit);
}

#[test]
fn return_outside_a_function_is_an_error() {
    assert_eq!(failure("return;"), ReturnOutsideFunction);
    assert_eq!(failure("if true { return 1; }"), ReturnOutsideFunction);
}

/// §4: `break` applies to the innermost enclosing loop, and a call is not
/// something it reaches across.
#[test]
fn break_does_not_cross_a_call() {
    assert_eq!(
        failure("fn escape() { break; } while true { escape(); }"),
        BreakOutsideLoop
    );
}

/// §5: recursion is permitted, and mutual recursion needs declarations to be
/// order-independent. §12 owes the general rule; hoisting per block is the
/// stand-in.
#[test]
fn functions_recurse_and_see_each_other() {
    assert_eq!(
        int("fn fact(n: u64) -> u64 { if n == 0 { 1 } else { n * fact(n - 1) } } fact(10)"),
        3628800
    );
    assert_eq!(
        int(
            "fn even(n: u64) -> bool { if n == 0 { true } else { odd(n - 1) } }
             fn odd(n: u64) -> bool { if n == 0 { false } else { even(n - 1) } }
             if even(10) { 1 } else { 0 }"
        ),
        1
    );
    assert_eq!(
        int("let answer = twice(21); fn twice(n: u64) -> u64 { n * 2 } answer"),
        42,
        "a declaration is visible above where it is written"
    );
}

/// §5: no tail-call guarantee, so deep recursion traps. Raised at a depth the
/// host stack can still afford, rather than by falling off it.
#[test]
fn runaway_recursion_traps() {
    assert_eq!(
        failure("fn forever(n: u64) -> u64 { forever(n) } forever(0)"),
        RecursionLimit
    );
}

/// §5 checks arity statically. There is no static anything yet, so it is
/// checked at the call — a stand-in for a compile error.
#[test]
fn arity_is_checked() {
    assert_eq!(
        failure("fn add(a: u64, b: u64) -> u64 { a + b } add(1)"),
        WrongArity {
            callee: "`add`".to_string(),
            expected: 2,
            found: 1,
        }
    );
    assert_eq!(failure("let x = 1; x()"), NotCallable("an integer"));
}

/// §5: one name, one function — no overloading, by arity or by type.
#[test]
fn a_function_is_a_value() {
    assert_eq!(
        int("fn add(a: u64, b: u64) -> u64 { a + b } let f = add; f(1, 2)"),
        3
    );
    assert_eq!(
        int("fn twice(n: u64) -> u64 { n * 2 }
             fn apply(g: fn(u64) -> u64, x: u64) -> u64 { g(x) }
             apply(twice, 21)"),
        42
    );
    assert_eq!(
        value("fn add(a: u64) -> u64 { a } add").to_string(),
        "<fn add>"
    );
}

/// §2 says nothing about comparing functions, so `==` refuses rather than
/// inventing an answer.
#[test]
fn functions_do_not_compare() {
    assert_eq!(
        failure("fn f() { } f == f"),
        BinaryTypeMismatch {
            operator: "==",
            left: "a function",
            right: "a function",
        }
    );
}

// ---- What a `fn` can see (§5) ----------------------------------------------

/// §5: a nested `fn` is scoped to its block and **captures nothing** — an
/// ordinary function that happens to be private. Keeping it capture-free is
/// what lets every `fn` compile to a plain wasm function with no environment.
#[test]
fn a_fn_captures_nothing() {
    assert_eq!(
        failure("let outer = 1; fn read() -> u64 { outer } read()"),
        NotCaptured("outer".to_string())
    );
    assert_eq!(
        failure("let mut outer = 1; fn write() { outer = 2; } write()"),
        NotCaptured("outer".to_string())
    );
}

#[test]
fn a_nested_fn_is_scoped_to_its_block() {
    assert_eq!(int("{ fn inner() -> u64 { 1 } inner() }"), 1);
    assert_eq!(
        failure("{ fn inner() -> u64 { 1 } }; inner()"),
        UnknownName("inner".to_string())
    );
}

// ---- Lambdas (§5) ----------------------------------------------------------

#[test]
fn a_lambda_is_a_value_and_calls() {
    assert_eq!(int("let inc = (x) -> x + 1; inc(41)"), 42);
    assert_eq!(int("let inc = (x: u64) -> x + 1; inc(41)"), 42);
    assert_eq!(int("let go = () -> 7; go()"), 7);
    assert_eq!(
        int("let f = (x) -> { let y = x * 2; y + 1 }; f(20)"),
        41,
        "a lambda body may be a block"
    );
    assert_eq!(value("let f = () -> 1; f").to_string(), "<lambda>");
}

/// §5: lambdas capture by reference, implicitly. Mutation of a captured binding
/// is visible to everyone holding it, and the binding must be `mut` to be
/// mutated at all (§3).
#[test]
fn a_lambda_captures_by_reference() {
    assert_eq!(int("let n = 1; let read = () -> n; read()"), 1);
    assert_eq!(
        int("let mut n = 1; let read = () -> n; n = 2; read()"),
        2,
        "by reference, so the later write is seen"
    );
    assert_eq!(
        int("let mut n = 1; let bump = () -> { n = n + 1; }; bump(); bump(); n"),
        3
    );
}

/// §5: captured bindings outlive the frame that created them.
#[test]
fn a_capture_outlives_its_frame() {
    assert_eq!(
        int("let make = () -> { let mut n = 0; () -> { n = n + 1; n } };
             let counter = make();
             counter();
             counter()"),
        2
    );
}

/// A lambda inherits its creator's barrier rather than raising a new one, so
/// one written inside a `fn` cannot reach what the `fn` itself couldn't.
#[test]
fn a_lambda_inside_a_fn_sees_no_further_than_the_fn() {
    assert_eq!(
        failure("let outer = 1; fn f() -> u64 { let g = () -> outer; g() } f()"),
        NotCaptured("outer".to_string())
    );
    assert_eq!(
        int("fn f(n: u64) -> u64 { let g = () -> n + 1; g() } f(41)"),
        42,
        "a parameter is the function's own, and is captured normally"
    );
}

// ---- `mut` parameters (§5) -------------------------------------------------

/// §5: `mut` on a parameter means the function may mutate the caller's value,
/// and the caller must pass a `mut` binding — so a call that mutates is visible
/// at the call site rather than only at the declaration.
#[test]
fn a_mut_parameter_writes_back_to_the_caller() {
    assert_eq!(
        int("fn advance(c: mut u64, by: u64) { c = c + by; }
             let mut tally = 0;
             advance(tally, 5);
             tally"),
        5
    );
}

#[test]
fn a_mut_parameter_needs_a_mut_binding() {
    assert_eq!(
        failure("fn advance(c: mut u64) { c = c + 1; } let frozen = 0; advance(frozen);"),
        MutArgumentNotMutable {
            parameter: "c".to_string(),
            argument: "frozen".to_string(),
        }
    );
    assert_eq!(
        failure("fn advance(c: mut u64) { c = c + 1; } advance(1 + 1);"),
        MutArgumentNotAPlace {
            parameter: "c".to_string(),
        }
    );
}

/// §5: parameters are immutable bindings by default, exactly like a `let`.
#[test]
fn a_plain_parameter_is_immutable() {
    assert_eq!(
        failure("fn f(n: u64) -> u64 { n = n + 1; n } f(1)"),
        ImmutableBinding("n".to_string())
    );
}

/// A half-parsed tree is what the language server wants and what an interpreter
/// does not: evaluating around an error node means guessing at the missing text.
#[test]
fn a_program_that_did_not_parse_does_not_run() {
    let Err(Error::Syntax(errors)) = run("let x = ;") else {
        panic!("`let x = ;` should not have run");
    };
    assert!(!errors.is_empty());
}

/// `parser::parse` drops the tokenizer's diagnostics, so this is the test that
/// keeps them from going missing here too.
#[test]
fn a_lexical_error_is_reported_even_when_the_parse_survives() {
    let Err(Error::Syntax(errors)) = run(r#"let x = "\q";"#) else {
        panic!("a bad escape should not have run");
    };
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].message.contains("escape"),
        "{}",
        errors[0].message
    );
}

// ---- Echoing a value -------------------------------------------------------

#[test]
fn values_display_as_they_are_echoed() {
    assert_eq!(value("42").to_string(), "42");
    assert_eq!(value("1.5").to_string(), "1.5");
    assert_eq!(
        value("2.0 + 1.0").to_string(),
        "3.0",
        "a float keeps its point"
    );
    assert_eq!(value("true").to_string(), "true");
    assert_eq!(value("()").to_string(), "()");
    assert_eq!(value("'x'").to_string(), "'x'");
    assert_eq!(
        value(r#""hi\n""#).to_string(),
        r#""hi\n""#,
        "a string is quoted, so it can't be mistaken for a name"
    );
}
