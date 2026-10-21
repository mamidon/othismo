//! Semantic tokens: what each token *is*, so a theme can colour it.
//!
//! This used to be a palette that cycled per token, plus a background tint for
//! nesting depth. Both were bring-up instruments: they answered "where does one
//! token end and the next begin" and "how deep am I", which is what you want
//! while a tokenizer and a parser are being written and nothing else can tell
//! you. Neither is what you want while writing Glue.
//!
//! So colour now says what every other language says with it — a name is a
//! variable, a parameter, a property, a type; a call is a function; a literal
//! is a string or a number. The types below are the **standard** LSP ones
//! rather than a private palette, which means the user's own theme colours
//! Glue the way it colours everything else, and the extension ships no hex
//! values at all.
//!
//! # Where a name's role comes from
//!
//! The token stream cannot tell a variable from a function: both are `Ident`.
//! The tree can, because the parser already made the distinction structural —
//! [`NodeKind::NamePat`] for a binding, [`NodeKind::Param`] for a parameter,
//! [`NodeKind::FieldExpr`] for a field, [`NodeKind::MethodCallExpr`] for a
//! method. Two roles need one step of context rather than the parent alone,
//! and they are the two the parser deliberately did *not* flatten: the callee
//! of a [`NodeKind::CallExpr`] and the type of a [`NodeKind::StructLitExpr`]
//! are ordinary [`NodeKind::NameExpr`]s, so the walk hands each a hint on the
//! way down.

use parser::{Child, NodeId, NodeKind, Tree};
use tokenizer::{Token, TokenKind};
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType};

use crate::line_index::LineIndex;

/// The legend, in the order the protocol indexes it. Standard types only —
/// every one of these is a type VS Code's built-in themes already style.
const TOKEN_TYPES: [SemanticTokenType; 12] = [
    SemanticTokenType::KEYWORD,
    SemanticTokenType::COMMENT,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::METHOD,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::TYPE,
    SemanticTokenType::STRUCT,
];

const KEYWORD: u32 = 0;
const COMMENT: u32 = 1;
const STRING: u32 = 2;
const NUMBER: u32 = 3;
const OPERATOR: u32 = 4;
const VARIABLE: u32 = 5;
const PARAMETER: u32 = 6;
const FUNCTION: u32 = 7;
const METHOD: u32 = 8;
const PROPERTY: u32 = 9;
const TYPE: u32 = 10;
const STRUCT: u32 = 11;

/// `declaration` marks the place a name is introduced rather than used, and
/// `defaultLibrary` marks §lexical's built-in type names — which are ordinary
/// identifiers, not keywords, so this is the only thing that sets `u64` apart
/// from a struct the program declared.
const TOKEN_MODIFIERS: [SemanticTokenModifier; 2] = [
    SemanticTokenModifier::DECLARATION,
    SemanticTokenModifier::DEFAULT_LIBRARY,
];

const DECLARATION: u32 = 1 << 0;
const DEFAULT_LIBRARY: u32 = 1 << 1;

/// The predeclared type names (§lexical, §types). Kept in step with
/// `elab::lower`'s prelude by hand; a name missing here is coloured as an
/// ordinary type, which is wrong but not misleading.
const BUILTIN_TYPES: [&str; 15] = [
    "bool", "char", "Str", "f32", "f64", "u8", "u16", "u32", "u64", "s8", "s16", "s32", "s64",
    "()", "Type",
];

pub fn token_types() -> Vec<SemanticTokenType> {
    TOKEN_TYPES.to_vec()
}

pub fn token_modifiers() -> Vec<SemanticTokenModifier> {
    TOKEN_MODIFIERS.to_vec()
}

/// A token's type and modifiers, as the protocol encodes them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Class {
    ty: u32,
    modifiers: u32,
}

impl Class {
    fn new(ty: u32) -> Class {
        Class { ty, modifiers: 0 }
    }

    fn declared(ty: u32) -> Class {
        Class {
            ty,
            modifiers: DECLARATION,
        }
    }
}

/// Every token in `tree`, encoded for `textDocument/semanticTokens/full`.
pub fn tokens(tree: &Tree, source: &str, index: &LineIndex) -> Vec<SemanticToken> {
    let mut flat = Vec::new();
    walk(tree, tree.root(), source, None, &mut flat);

    let mut encoded = Vec::new();
    let (mut previous_line, mut previous_start) = (0, 0);

    for (token, class) in flat {
        // A semantic token may not span lines, so a multiline block comment
        // becomes one token per line it covers.
        for (line, start, length) in lines_of(token, index) {
            let delta_line = line - previous_line;
            let delta_start = if delta_line == 0 {
                start - previous_start
            } else {
                start
            };
            encoded.push(SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type: class.ty,
                token_modifiers_bitset: class.modifiers,
            });
            (previous_line, previous_start) = (line, start);
        }
    }

    encoded
}

/// Tokens in source order, each with the role its position in the tree gives
/// it.
///
/// `hint` is the role this node's own name token takes when the node alone
/// cannot say — see the module docs. It reaches one level only, so `(f)(1)`
/// leaves `f` a variable rather than colouring the parenthesis's contents.
fn walk(tree: &Tree, node: NodeId, source: &str, hint: Option<u32>, out: &mut Vec<(Token, Class)>) {
    let kind = tree.kind(node);
    let mut first_child_node = true;

    for child in tree.children(node) {
        match child {
            Child::Node(child) => {
                let inherited = match kind {
                    // `f(…)` — the callee is a plain `NameExpr`, because
                    // §expressions' postfix rung wraps rather than flattens.
                    NodeKind::CallExpr if first_child_node => Some(FUNCTION),
                    // `Point { … }` — likewise the type name.
                    NodeKind::StructLitExpr if first_child_node => Some(STRUCT),
                    _ => None,
                };
                first_child_node = false;
                walk(tree, child, source, inherited, out);
            }
            Child::Token(token) => {
                if let Some(class) = classify(kind, token, source, hint) {
                    out.push((token, class));
                }
            }
        }
    }
}

/// What a single token is, given the node holding it.
fn classify(parent: NodeKind, token: Token, source: &str, hint: Option<u32>) -> Option<Class> {
    let ty = match token.kind {
        // No glyph to colour, and nothing to say about it.
        TokenKind::Whitespace | TokenKind::Eof => return None,
        TokenKind::LineComment | TokenKind::BlockComment => COMMENT,
        TokenKind::Str | TokenKind::Char => STRING,
        TokenKind::Int | TokenKind::Float => NUMBER,
        // Left uncoloured on purpose: a diagnostic already underlines it, and
        // giving it a role would dress up text the lexer could not read.
        TokenKind::Unknown => return None,
        TokenKind::Ident => return Some(name_class(parent, token, source, hint)),
        kind if kind.is_keyword() => KEYWORD,
        // Punctuation and operators alike. Most themes leave these at the
        // foreground colour, which is the right amount of attention for them.
        _ => OPERATOR,
    };
    Some(Class::new(ty))
}

/// What an identifier is, which only the tree knows.
fn name_class(parent: NodeKind, token: Token, source: &str, hint: Option<u32>) -> Class {
    match parent {
        NodeKind::FnDecl => Class::declared(FUNCTION),
        NodeKind::StructDecl => Class::declared(STRUCT),
        NodeKind::TypeAliasDecl => Class::declared(TYPE),
        NodeKind::Param | NodeKind::LambdaParam => Class::declared(PARAMETER),
        NodeKind::FieldDecl => Class::declared(PROPERTY),
        NodeKind::NamePat => Class::declared(VARIABLE),
        NodeKind::FieldInit | NodeKind::FieldExpr => Class::new(PROPERTY),
        NodeKind::MethodCallExpr => Class::new(METHOD),
        NodeKind::NameType => {
            let text = &source[token.span.start as usize..token.span.end as usize];
            if BUILTIN_TYPES.contains(&text) {
                Class {
                    ty: TYPE,
                    modifiers: DEFAULT_LIBRARY,
                }
            } else {
                Class::new(TYPE)
            }
        }
        // A bare name is a variable unless the shape around it said otherwise.
        NodeKind::NameExpr => Class::new(hint.unwrap_or(VARIABLE)),
        // An identifier the grammar did not place — inside an error node, say.
        _ => Class::new(VARIABLE),
    }
}

/// The `(line, start column, length)` of each line `token` covers.
fn lines_of(token: Token, index: &LineIndex) -> Vec<(u32, u32, u32)> {
    let (first_line, first_column) = index.line_col(token.span.start as usize);
    let (last_line, last_column) = index.line_col(token.span.end as usize);

    if first_line == last_line {
        return vec![(first_line, first_column, last_column - first_column)];
    }

    let mut pieces = Vec::new();
    for line in first_line..=last_line {
        let start = index.line_start(line);
        let end = index.line_start(line + 1);
        let (from, to) = match line {
            _ if line == first_line => (token.span.start as usize, end),
            _ if line == last_line => (start, token.span.end as usize),
            _ => (start, end),
        };
        // The newline itself isn't part of any token's visible extent.
        let to = to.saturating_sub(usize::from(line != last_line));
        if to > from {
            pieces.push((line, (from - start) as u32, (to - from) as u32));
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every token's text, type, and modifiers, in source order.
    fn roles(source: &str) -> Vec<(String, u32, u32)> {
        let parsed = parser::parse(source);
        let mut flat = Vec::new();
        walk(&parsed.tree, parsed.tree.root(), source, None, &mut flat);
        flat.into_iter()
            .map(|(token, class)| {
                let text = source[token.span.start as usize..token.span.end as usize].to_string();
                (text, class.ty, class.modifiers)
            })
            .collect()
    }

    /// The first token spelled `text`.
    #[track_caller]
    fn role(source: &str, text: &str) -> (u32, u32) {
        roles(source)
            .into_iter()
            .find(|(spelling, ..)| spelling == text)
            .map(|(_, ty, modifiers)| (ty, modifiers))
            .unwrap_or_else(|| panic!("no token {text:?} in {source:?}"))
    }

    #[test]
    fn a_binding_declares_a_variable_and_a_use_is_a_variable() {
        assert_eq!(role("let x = 1;", "x"), (VARIABLE, DECLARATION));
        assert_eq!(role("y + 1", "y"), (VARIABLE, 0));
    }

    /// The callee of a call is a `NameExpr` like any other, so this is the
    /// hint the walk passes down rather than anything the parent says.
    #[test]
    fn a_callee_is_a_function() {
        assert_eq!(role("f(1)", "f"), (FUNCTION, 0));
        assert_eq!(role("fn f() { }", "f"), (FUNCTION, DECLARATION));
    }

    /// One level only: the name inside a parenthesis stays a variable.
    #[test]
    fn the_hint_does_not_pass_through_a_paren() {
        assert_eq!(role("(f)(1)", "f"), (VARIABLE, 0));
    }

    #[test]
    fn parameters_are_parameters_where_declared_and_variables_where_used() {
        let source = "fn f(a: u64) -> u64 { a }";
        let tokens = roles(source);
        let names: Vec<_> = tokens.iter().filter(|(text, ..)| text == "a").collect();
        assert_eq!(names.len(), 2);
        assert_eq!((names[0].1, names[0].2), (PARAMETER, DECLARATION));
        assert_eq!((names[1].1, names[1].2), (VARIABLE, 0));
    }

    #[test]
    fn structs_fields_and_methods() {
        assert_eq!(role("struct P { x: u64 }", "P"), (STRUCT, DECLARATION));
        assert_eq!(role("struct P { x: u64 }", "x"), (PROPERTY, DECLARATION));
        // A literal's type name, and its field names.
        assert_eq!(role("P { x: 1 }", "P"), (STRUCT, 0));
        assert_eq!(role("P { x: 1 }", "x"), (PROPERTY, 0));
        // Access, and the call form the parser keeps separate from it.
        assert_eq!(role("p.x", "x"), (PROPERTY, 0));
        assert_eq!(role("p.go()", "go"), (METHOD, 0));
    }

    /// §lexical's type names are ordinary identifiers rather than keywords,
    /// so `defaultLibrary` is the only thing telling `u64` from `Handle`.
    #[test]
    fn a_builtin_type_is_marked_and_a_declared_one_is_not() {
        assert_eq!(role("let x: u64 = 1;", "u64"), (TYPE, DEFAULT_LIBRARY));
        assert_eq!(role("let x: Handle = h;", "Handle"), (TYPE, 0));
        assert_eq!(role("type Handle = u64;", "Handle"), (TYPE, DECLARATION));
    }

    #[test]
    fn literals_keywords_and_comments() {
        assert_eq!(role("\"hi\"", "\"hi\""), (STRING, 0));
        assert_eq!(role("'c'", "'c'"), (STRING, 0));
        assert_eq!(role("42u8", "42u8"), (NUMBER, 0));
        assert_eq!(role("1.5", "1.5"), (NUMBER, 0));
        assert_eq!(role("true", "true"), (KEYWORD, 0));
        assert_eq!(role("if x { }", "if"), (KEYWORD, 0));
        assert_eq!(role("1 + 2", "+"), (OPERATOR, 0));
        assert_eq!(role("// hi", "// hi"), (COMMENT, 0));
    }

    /// Whitespace has no glyph to colour; a comment does.
    #[test]
    fn whitespace_is_left_out() {
        assert_eq!(roles("a + b").len(), 3);
        assert_eq!(roles("a // hi").len(), 2);
    }

    fn encoded(source: &str) -> Vec<(u32, u32, u32)> {
        tokens(&parser::parse(source).tree, source, &LineIndex::new(source))
            .iter()
            .map(|token| (token.delta_line, token.delta_start, token.length))
            .collect()
    }

    /// Positions are deltas from the previous token, per the protocol.
    #[test]
    fn positions_are_relative() {
        assert_eq!(encoded("a+b"), [(0, 0, 1), (0, 1, 1), (0, 1, 1)]);
    }

    /// A token may not span lines, so one that does is split — and the split
    /// pieces exclude the newline itself.
    #[test]
    fn multiline_tokens_are_split_per_line() {
        assert_eq!(encoded("1 /*x\ny*/"), [(0, 0, 1), (0, 2, 3), (1, 0, 3)]);
    }
}
