//! Semantic tokens: the syntax tree, made visible.
//!
//! Not syntax highlighting. Highlighting colors a token by what it *is* — a
//! keyword, a string — and none of that would tell you whether the parser
//! agreed with you. This colors a token by **how deeply nested it is**, the way
//! bracket-pair colorization does, so the shape on screen is the shape of the
//! tree and a precedence mistake is something you can see rather than something
//! you have to dump the tree to find.
//!
//! Depth alone leaves runs of same-colored tokens — the `,` and `)` of an
//! argument list sit at one depth and blur together. So the one remaining
//! channel, a modifier the theme underlines, alternates token by token. Colour
//! says where you are in the tree; the underline says where one token ends and
//! the next begins.

use parser::{Child, NodeId, Tree};
use tokenizer::{Token, TokenKind};
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType};

use crate::line_index::LineIndex;

/// The palette, by name. Each of these is styled in the extension's
/// `package.json`; adding one here means adding a colour there.
const DEPTH_TYPES: [&str; 6] = [
    "glueDepth0",
    "glueDepth1",
    "glueDepth2",
    "glueDepth3",
    "glueDepth4",
    "glueDepth5",
];

/// How many depths get their own colour before the palette repeats.
pub const DEPTHS: u32 = DEPTH_TYPES.len() as u32;

/// Bit 0 of the modifier set: on for every other token, off for the rest.
const ALTERNATE: u32 = 1;

pub fn token_types() -> Vec<SemanticTokenType> {
    DEPTH_TYPES
        .iter()
        .copied()
        .map(SemanticTokenType::new)
        .collect()
}

pub fn token_modifiers() -> Vec<SemanticTokenModifier> {
    vec![SemanticTokenModifier::new("glueBoundary")]
}

/// Every token in `tree`, encoded for `textDocument/semanticTokens/full`.
pub fn tokens(tree: &Tree, index: &LineIndex) -> Vec<SemanticToken> {
    let mut flat = Vec::new();
    walk(tree, tree.root(), 0, &mut flat);

    let mut encoded = Vec::new();
    let (mut previous_line, mut previous_start) = (0, 0);

    for (order, (token, depth)) in flat.iter().enumerate() {
        let modifiers = if order % 2 == 0 { 0 } else { ALTERNATE };
        let token_type = depth % DEPTHS;

        // A semantic token may not span lines, so a multiline string or block
        // comment becomes one token per line it covers.
        for (line, start, length) in lines_of(*token, index) {
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
                token_type,
                token_modifiers_bitset: modifiers,
            });
            (previous_line, previous_start) = (line, start);
        }
    }

    encoded
}

/// Tokens in source order, each with the number of nodes enclosing it.
///
/// Whitespace is left out — it has no depth worth seeing and would only cost
/// the client work. Comments stay in, since where a comment attached is exactly
/// the sort of thing this view is for.
fn walk(tree: &Tree, node: NodeId, depth: u32, out: &mut Vec<(Token, u32)>) {
    for child in tree.children(node) {
        match child {
            Child::Node(child) => walk(tree, child, depth + 1, out),
            Child::Token(token) if token.kind != TokenKind::Whitespace => out.push((token, depth)),
            Child::Token(_) => {}
        }
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

    fn of(source: &str) -> Vec<SemanticToken> {
        tokens(
            &parser::parse_expression(source).tree,
            &LineIndex::new(source),
        )
    }

    /// `a+b*c` nests `b*c` one level deeper than `a`, and the colours say so.
    #[test]
    fn depth_is_the_colour() {
        let types: Vec<_> = of("a+b*c").iter().map(|t| t.token_type).collect();
        //                     a  +  b  *  c
        assert_eq!(types, [2, 1, 3, 2, 3]);
    }

    /// Adjacent tokens always differ in colour, underline, or both.
    #[test]
    fn adjacent_tokens_are_distinguishable() {
        for source in ["a+b*c", "f(a,b)", "a.b.c", "(a)"] {
            let encoded = of(source);
            for pair in encoded.windows(2) {
                assert!(
                    pair[0].token_type != pair[1].token_type
                        || pair[0].token_modifiers_bitset != pair[1].token_modifiers_bitset,
                    "{source:?} has two indistinguishable tokens in a row"
                );
            }
        }
    }

    /// Positions are deltas from the previous token, per the protocol.
    #[test]
    fn positions_are_relative() {
        let encoded = of("a+b");
        assert_eq!(
            encoded
                .iter()
                .map(|t| (t.delta_line, t.delta_start, t.length))
                .collect::<Vec<_>>(),
            [(0, 0, 1), (0, 1, 1), (0, 1, 1)]
        );
    }

    /// A token may not span lines, so one that does is split — and the split
    /// pieces exclude the newline itself.
    #[test]
    fn multiline_tokens_are_split_per_line() {
        let encoded = of("1 /*x\ny*/");
        assert_eq!(
            encoded
                .iter()
                .map(|t| (t.delta_line, t.delta_start, t.length))
                .collect::<Vec<_>>(),
            [(0, 0, 1), (0, 2, 3), (1, 0, 3)]
        );
    }

    /// Whitespace carries no depth worth seeing; comments do.
    #[test]
    fn whitespace_is_left_out() {
        assert_eq!(of("a + b").len(), 3);
        assert_eq!(of("a // hi").len(), 2);
    }
}
