use std::borrow::Cow;

use crate::{DiagnosticKind, Literal, NumericType, Token, TokenKind, Tokens, literal_value, tokenize};

// --- Helpers ---------------------------------------------------------------

/// The significant tokens' kinds, without the trailing `Eof`.
fn kinds(source: &str) -> Vec<TokenKind> {
    let lexed = tokenize(source);
    lexed
        .significant()
        .map(|token| token.kind)
        .filter(|kind| *kind != TokenKind::Eof)
        .collect()
}

fn all_kinds(source: &str) -> Vec<TokenKind> {
    tokenize(source).tokens.iter().map(|t| t.kind).collect()
}

fn errors(source: &str) -> Vec<DiagnosticKind> {
    tokenize(source)
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.kind)
        .collect()
}

fn first_literal(source: &str) -> Option<Literal<'_>> {
    let lexed = tokenize(source);
    let token = lexed.significant().next().expect("no significant token");
    literal_value(token, source)
}

/// Both load-bearing invariants: spans tile the source with no gaps and no
/// overlaps, and concatenating them reproduces the file byte for byte.
#[track_caller]
fn assert_lossless(source: &str) {
    let lexed: Tokens = tokenize(source);
    let mut offset = 0;
    for token in &lexed.tokens {
        assert_eq!(
            token.span.start as usize, offset,
            "gap or overlap before {token:?} in {source:?}"
        );
        offset = token.span.end as usize;
    }
    assert_eq!(offset, source.len(), "tokens stop short of the end");

    let rebuilt: String = lexed
        .tokens
        .iter()
        .map(|token: &Token| token.text(source))
        .collect();
    assert_eq!(rebuilt, source, "token stream is not lossless");

    assert_eq!(
        lexed.tokens.last().map(|token| token.kind),
        Some(TokenKind::Eof)
    );
}

// --- Losslessness and totality ---------------------------------------------

#[test]
fn token_spans_tile_the_source() {
    for source in [
        "",
        "   ",
        "let x = 1;",
        "// comment\nlet x = 1; /* block */ x",
        "\"string\" 'c' r\"raw\" \"\"\"multi\nline\"\"\"",
        "0xff 1_000 .5 1e10 1.0f32",
        "a.b().c[0] |> ??? é 🎉",
        "\u{feff}let x = 1;",
        "\"unterminated",
        "/* unterminated",
    ] {
        assert_lossless(source);
    }
}

#[test]
fn lexes_the_example_program() {
    let source = include_str!("../../examples/hello.glue");
    assert_lossless(source);
    assert_eq!(tokenize(source).diagnostics, Vec::new());
}

#[test]
fn empty_source_is_just_eof() {
    assert_eq!(all_kinds(""), [TokenKind::Eof]);
}

#[test]
fn every_truncation_of_a_gnarly_program_lexes() {
    // Half-typed programs are the language server's normal case, so every
    // prefix has to survive — the progress invariant is what makes that true,
    // and a truncation is how you find the arm that forgot it.
    let source = "let x = \"a\\u{1F600}\" + r\"b\" + \"\"\"c\"\"\" + 'd' + 0x1_f /* /**/ */ .5 é@";
    for end in 0..=source.len() {
        if source.is_char_boundary(end) {
            assert_lossless(&source[..end]);
        }
    }
}

// --- Identifiers and keywords ----------------------------------------------

#[test]
fn identifiers_and_keywords() {
    use TokenKind::*;
    assert_eq!(kinds("let mut fn struct type"), [Let, Mut, Fn, Struct, Type]);
    assert_eq!(
        kinds("return if else while for in break continue match"),
        [Return, If, Else, While, For, In, Break, Continue, Match]
    );
    assert_eq!(kinds("import export true false as"), [Import, Export, True, False, As]);
    assert_eq!(kinds("_ _x x9 X_9 letx r"), [Ident; 6]);
}

#[test]
fn primitive_type_names_are_identifiers_not_keywords() {
    // §6 makes `u64` and `Str` types, resolved by name. Reserving them would
    // be a language decision nobody has made.
    assert_eq!(kinds("u64 s32 f64 bool Str char"), [TokenKind::Ident; 6]);
}

// --- Operators and punctuation ---------------------------------------------

#[test]
fn operators() {
    use TokenKind::*;
    let cases: &[(&str, TokenKind)] = &[
        ("(", LParen),
        (")", RParen),
        ("{", LBrace),
        ("}", RBrace),
        ("[", LBracket),
        ("]", RBracket),
        (",", Comma),
        (";", Semicolon),
        (":", Colon),
        ("::", ColonColon),
        (".", Dot),
        ("..", DotDot),
        ("...", DotDotDot),
        ("->", Arrow),
        ("+", Plus),
        ("-", Minus),
        ("*", Star),
        ("/", Slash),
        ("%", Percent),
        ("&", Amp),
        ("|", Pipe),
        ("^", Caret),
        ("~", Tilde),
        ("!", Bang),
        ("&&", AmpAmp),
        ("||", PipePipe),
        ("<<", Shl),
        (">>", Shr),
        ("=", Eq),
        ("==", EqEq),
        ("!=", BangEq),
        ("<", Lt),
        ("<=", Le),
        (">", Gt),
        (">=", Ge),
        ("+=", PlusEq),
        ("-=", MinusEq),
        ("*=", StarEq),
        ("/=", SlashEq),
        ("%=", PercentEq),
        ("&=", AmpEq),
        ("|=", PipeEq),
        ("^=", CaretEq),
        ("<<=", ShlEq),
        (">>=", ShrEq),
    ];
    for (source, expected) in cases {
        assert_eq!(kinds(source), [*expected], "lexing {source:?}");
    }
}

#[test]
fn takes_the_longest_operator() {
    use TokenKind::*;
    assert_eq!(kinds("<<=<<<=<"), [ShlEq, Shl, Le, Lt]);
    assert_eq!(kinds(">>=>>>=>"), [ShrEq, Shr, Ge, Gt]);
    assert_eq!(kinds("->-=-"), [Arrow, MinusEq, Minus]);
    assert_eq!(kinds("....."), [DotDotDot, DotDot]);
    assert_eq!(kinds("&&&"), [AmpAmp, Amp]);
    assert_eq!(kinds("|||"), [PipePipe, Pipe]);
}

// --- Comments --------------------------------------------------------------

#[test]
fn comments() {
    use TokenKind::*;
    assert_eq!(all_kinds("// x"), [LineComment, Eof]);
    assert_eq!(all_kinds("/* x */"), [BlockComment, Eof]);
    assert_eq!(all_kinds("/// x"), [DocComment, Eof]);
    // A row of slashes is a divider, not a doc comment attached to nothing.
    assert_eq!(all_kinds("//// x"), [LineComment, Eof]);
    assert_eq!(all_kinds("/////"), [LineComment, Eof]);
}

#[test]
fn block_comments_nest() {
    use TokenKind::*;
    assert_eq!(all_kinds("/* a /* b */ c */"), [BlockComment, Eof]);
    assert_eq!(all_kinds("/**/x"), [BlockComment, Ident, Eof]);
    assert_eq!(all_kinds("/*/ */x"), [BlockComment, Ident, Eof]);
    assert_eq!(errors("/* a /* b */"), [DiagnosticKind::UnterminatedBlockComment]);
}

#[test]
fn doc_comments_are_not_trivia() {
    // They attach to the following declaration (§1), so the parser sees them.
    assert_eq!(kinds("/// doc\nfn f() {}"), {
        use TokenKind::*;
        vec![DocComment, Fn, Ident, LParen, RParen, LBrace, RBrace]
    });
}

// --- Numbers ---------------------------------------------------------------

#[test]
fn integer_literals() {
    assert_eq!(kinds("0 42 1_000_000"), [TokenKind::Int; 3]);
    assert_eq!(kinds("0xff 0o17 0b1010"), [TokenKind::Int; 3]);
    assert_eq!(
        first_literal("0xff"),
        Some(Literal::Int {
            value: 255,
            suffix: None
        })
    );
    assert_eq!(
        first_literal("0b1010"),
        Some(Literal::Int {
            value: 10,
            suffix: None
        })
    );
    assert_eq!(
        first_literal("0o17"),
        Some(Literal::Int {
            value: 15,
            suffix: None
        })
    );
    assert_eq!(
        first_literal("1_000u32"),
        Some(Literal::Int {
            value: 1000,
            suffix: Some(NumericType::U32)
        })
    );
}

#[test]
fn float_literals() {
    for source in ["1.5", ".5", "1e10", "1E-3", "1.0f32", "1.5e-3"] {
        assert_eq!(kinds(source), [TokenKind::Float], "lexing {source:?}");
    }
    assert_eq!(
        first_literal(".5"),
        Some(Literal::Float {
            value: 0.5,
            suffix: None
        })
    );
    assert_eq!(
        first_literal("1e10"),
        Some(Literal::Float {
            value: 1e10,
            suffix: None
        })
    );
    assert_eq!(
        first_literal("1.0f32"),
        Some(Literal::Float {
            value: 1.0,
            suffix: Some(NumericType::F32)
        })
    );
}

#[test]
fn a_trailing_dot_is_not_part_of_a_literal() {
    // §1: `1.` is not a float, which is what keeps `1.method()` free of
    // lookahead.
    use TokenKind::*;
    assert_eq!(kinds("1."), [Int, Dot]);
    assert_eq!(kinds("1.method()"), [Int, Dot, Ident, LParen, RParen]);
}

#[test]
fn hex_is_integer_only() {
    // `f`, `3`, and `2` are all hex digits, so `0x1f32` has no suffix — while
    // `0x1u8` does.
    assert_eq!(
        first_literal("0x1f32"),
        Some(Literal::Int {
            value: 0x1f32,
            suffix: None
        })
    );
    assert_eq!(
        first_literal("0x1u8"),
        Some(Literal::Int {
            value: 1,
            suffix: Some(NumericType::U8)
        })
    );
}

#[test]
fn an_integer_may_carry_a_float_suffix() {
    assert_eq!(
        first_literal("1f32"),
        Some(Literal::Int {
            value: 1,
            suffix: Some(NumericType::F32)
        })
    );
    assert_eq!(errors("1f32"), []);
}

#[test]
fn malformed_numbers() {
    use DiagnosticKind::*;
    assert_eq!(errors("0x"), [MissingDigits]);
    assert_eq!(errors("0b"), [MissingDigits]);
    assert_eq!(errors("1_"), [TrailingUnderscore]);
    assert_eq!(errors("0x_1"), [LeadingUnderscore]);
    assert_eq!(errors("0X1F"), [UppercaseRadixPrefix]);
    assert_eq!(errors("1blah"), [UnknownSuffix]);
    assert_eq!(errors("1.0u8"), [FloatWithIntegerSuffix]);
    assert_eq!(errors("1e400"), [FloatOutOfRange]);
    assert_eq!(
        errors("999999999999999999999999999999999999999999"),
        [IntegerTooLarge]
    );
    // Separators may repeat between digits; only the boundaries are ruled out.
    assert_eq!(errors("1__0"), []);
}

#[test]
fn an_e_that_is_not_an_exponent_becomes_a_suffix() {
    // No backtracking: the `e` is left where it is rather than consumed and
    // given back.
    assert_eq!(kinds("1e"), [TokenKind::Int]);
    assert_eq!(errors("1e"), [DiagnosticKind::UnknownSuffix]);
}

// --- The `.5` rule ---------------------------------------------------------

#[test]
fn leading_dot_floats_follow_left_context() {
    use TokenKind::*;
    // The table from §1, verbatim.
    assert_eq!(kinds(".5"), [Float]);
    assert_eq!(kinds("f(.5)"), [Ident, LParen, Float, RParen]);
    assert_eq!(kinds("a + .5"), [Ident, Plus, Float]);
    assert_eq!(kinds("pair.0"), [Ident, Dot, Int]);
    assert_eq!(kinds("(a, b).0"), [LParen, Ident, Comma, Ident, RParen, Dot, Int]);
}

#[test]
fn whitespace_does_not_rescue_the_ambiguity() {
    // §1: decided by the preceding *token*, not by adjacency.
    use TokenKind::*;
    assert_eq!(kinds("pair. 0"), [Ident, Dot, Int]);
    assert_eq!(kinds("pair .0"), [Ident, Dot, Int]);
    // A comment between them changes nothing.
    assert_eq!(kinds("pair /* c */ .0"), [Ident, Dot, Int]);

    // The same rule with no expression in sight: two literals side by side
    // aren't valid syntax anyway, but the lexer still answers by left context,
    // so this is `1.5` `.` `5` and not two floats. The parser will reject it —
    // the point is that it rejects it for a reason the rule explains.
    assert_eq!(kinds("1.5 .5"), [Float, Dot, Int]);
}

#[test]
fn ranges_lex_before_the_dot_rule_runs() {
    use TokenKind::*;
    assert_eq!(kinds("0..5"), [Int, DotDot, Int]);
    assert_eq!(kinds("..5"), [DotDot, Int]);
}

#[test]
fn a_block_can_end_an_expression() {
    use TokenKind::*;
    assert_eq!(kinds("}.0"), [RBrace, Dot, Int]);
    assert_eq!(kinds("].0"), [RBracket, Dot, Int]);
    assert_eq!(kinds("true.0"), [True, Dot, Int]);
    // A keyword that cannot end an expression leaves the `.` to the float.
    assert_eq!(kinds("return .5"), [Return, Float]);
}

// --- Strings and characters ------------------------------------------------

#[test]
fn string_forms() {
    use TokenKind::*;
    assert_eq!(kinds("\"a\""), [Str]);
    assert_eq!(kinds("r\"a\""), [RawStr]);
    assert_eq!(kinds("\"\"\"a\"\"\""), [MultilineStr]);
    assert_eq!(kinds("'a'"), [Char]);
    // `""` is an empty string, not the start of a multi-line one.
    assert_eq!(kinds("\"\" x"), [Str, Ident]);
}

#[test]
fn string_values() {
    assert_eq!(first_literal("\"abc\""), Some(Literal::Str(Cow::Borrowed("abc"))));
    assert_eq!(
        first_literal("\"a\\nb\""),
        Some(Literal::Str(Cow::Owned("a\nb".to_string())))
    );
    assert_eq!(
        first_literal("\"\\u{1F600}\""),
        Some(Literal::Str(Cow::Owned("\u{1F600}".to_string())))
    );
    // Raw strings process nothing.
    assert_eq!(
        first_literal("r\"a\\nb\""),
        Some(Literal::Str(Cow::Borrowed("a\\nb")))
    );
    // A `"""` literal keeps its newlines and strips no indentation (§1).
    assert_eq!(
        first_literal("\"\"\"a\n  b\"\"\""),
        Some(Literal::Str(Cow::Borrowed("a\n  b")))
    );
}

#[test]
fn escape_free_strings_borrow() {
    assert!(matches!(
        first_literal("\"abc\""),
        Some(Literal::Str(Cow::Borrowed(_)))
    ));
}

#[test]
fn character_values() {
    assert_eq!(first_literal("'a'"), Some(Literal::Char('a')));
    assert_eq!(first_literal("'\\n'"), Some(Literal::Char('\n')));
    assert_eq!(first_literal("'\\u{1F600}'"), Some(Literal::Char('\u{1F600}')));
    assert_eq!(first_literal("'é'"), Some(Literal::Char('é')));
}

#[test]
fn boolean_literals_decode() {
    assert_eq!(first_literal("true"), Some(Literal::Bool(true)));
    assert_eq!(first_literal("false"), Some(Literal::Bool(false)));
}

#[test]
fn unterminated_strings_stop_at_the_newline() {
    use TokenKind::*;
    // One missing quote must not swallow the rest of the file.
    assert_eq!(kinds("\"abc\nlet x = 1;"), [Str, Let, Ident, Eq, Int, Semicolon]);
    assert_eq!(
        tokenize("\"abc\nlet x = 1;").diagnostics[0].kind,
        DiagnosticKind::UnterminatedString
    );
    assert_eq!(errors("r\"abc\nx"), [DiagnosticKind::UnterminatedRawString]);
    assert_eq!(errors("'a\nx"), [DiagnosticKind::UnterminatedChar]);
    // A `"""` string legitimately spans lines, so it runs to the end.
    assert_eq!(
        errors("\"\"\"abc\nlet x = 1;"),
        [DiagnosticKind::UnterminatedMultilineString]
    );
}

#[test]
fn escaped_quotes_do_not_terminate() {
    use TokenKind::*;
    assert_eq!(kinds("\"a\\\"b\" x"), [Str, Ident]);
    assert_eq!(kinds("\"\"\"a\\\"\"\"b\"\"\" x"), [MultilineStr, Ident]);
}

#[test]
fn character_literal_arity() {
    assert_eq!(errors("''"), [DiagnosticKind::EmptyChar]);
    assert_eq!(errors("'ab'"), [DiagnosticKind::OverlongChar]);
    assert_eq!(first_literal("'ab'"), None);
}

#[test]
fn bad_escapes_are_reported_where_they_are() {
    use DiagnosticKind::*;
    assert_eq!(errors("\"a\\qb\""), [UnknownEscape]);
    assert_eq!(errors("\"\\u{D800}\""), [UnicodeEscapeInvalidScalar]);
    assert_eq!(errors("\"\\u{}\""), [UnicodeEscapeEmpty]);
    assert_eq!(errors("\"\\u41\""), [UnicodeEscapeMissingBrace]);

    // The span covers the escape, not the string.
    let source = "\"ab\\qcd\"";
    let diagnostic = tokenize(source).diagnostics[0];
    assert_eq!(diagnostic.span.text(source), "\\q");

    // A bad escape doesn't cost you the rest of the string.
    assert_eq!(
        first_literal("\"a\\qb\""),
        Some(Literal::Str(Cow::Owned("ab".to_string())))
    );
}

// --- Source text -----------------------------------------------------------

#[test]
fn a_leading_byte_order_mark_is_skipped() {
    use TokenKind::*;
    assert_eq!(kinds("\u{feff}let"), [Let]);
    // But it is still covered by a token, so nothing is lost.
    assert_eq!(all_kinds("\u{feff}let"), [Whitespace, Let, Eof]);
}

#[test]
fn crlf_is_whitespace_and_normalizes_only_in_values() {
    use TokenKind::*;
    assert_eq!(kinds("let\r\nx"), [Let, Ident]);
    // Spans stay pointed at the untouched buffer; the decoded value is LF.
    assert_eq!(
        first_literal("\"\"\"a\r\nb\"\"\""),
        Some(Literal::Str(Cow::Owned("a\nb".to_string())))
    );
}

#[test]
fn non_ascii_identifiers_are_one_error() {
    use DiagnosticKind::*;
    let source = "café";
    assert_eq!(kinds(source), [TokenKind::Unknown]);
    assert_eq!(errors(source), [NonAsciiIdentifier]);
    let diagnostic = tokenize(source).diagnostics[0];
    assert_eq!(diagnostic.span.text(source), "café");

    // Starting with a non-ASCII letter lands in the same place.
    assert_eq!(errors("étude"), [NonAsciiIdentifier]);
}

#[test]
fn stray_characters_coalesce() {
    use DiagnosticKind::*;
    assert_eq!(kinds("@#$"), [TokenKind::Unknown]);
    assert_eq!(errors("@#$"), [UnexpectedCharacter]);
    // But they don't swallow what follows.
    assert_eq!(kinds("@let"), [TokenKind::Unknown, TokenKind::Let]);
    assert_eq!(errors("@ @"), [UnexpectedCharacter, UnexpectedCharacter]);
    // Non-breaking space is not whitespace, and saying so is the point.
    assert_eq!(errors("a\u{a0}b"), [UnexpectedCharacter]);
}

#[test]
fn text_inside_strings_and_comments_is_left_alone() {
    assert_eq!(errors("\"héllo 🎉\" // café"), []);
    assert_eq!(errors("/* 🎉 */"), []);
    assert_eq!(
        first_literal("\"héllo 🎉\""),
        Some(Literal::Str(Cow::Borrowed("héllo 🎉")))
    );
}

// --- The API ---------------------------------------------------------------

#[test]
fn significant_skips_trivia_but_not_doc_comments() {
    use TokenKind::*;
    let lexed = tokenize("  // c\n/// d\nlet");
    assert_eq!(
        lexed.significant().map(|t| t.kind).collect::<Vec<_>>(),
        [DocComment, Let, Eof]
    );
}

#[test]
fn literal_value_declines_non_literals() {
    let source = "let";
    let token = tokenize(source).significant().next().unwrap();
    assert_eq!(literal_value(token, source), None);
}
