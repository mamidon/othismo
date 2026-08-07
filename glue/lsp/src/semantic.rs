//! Semantic tokens: where one token ends and the next begins.
//!
//! Not syntax highlighting, and no longer depth either. Colour is a *nominal*
//! channel — six hues say "different", never "deeper", so reading nesting off
//! them meant decoding a legend in your head. Depth moved to the background
//! tint the extension paints, where brightness is ordinal and reads as depth
//! without being explained.
//!
//! That leaves the text colour free for what it is actually good at:
//! distinguishing one token from its neighbour. The palette cycles per token,
//! so adjacent tokens never share a colour and lexing mistakes — a `>>` that
//! should have been two `>`, a suffix swallowed into a literal — are visible
//! rather than inferred.

use parser::{Child, NodeId, Tree};
use tokenizer::{Token, TokenKind};
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType};

use crate::line_index::LineIndex;

/// The palette, by name. Each of these is styled in the extension's
/// `package.json`; adding one here means adding a colour there.
const TOKEN_TYPES: [&str; 8] = [
    "glueTok0", "glueTok1", "glueTok2", "glueTok3", "glueTok4", "glueTok5", "glueTok6", "glueTok7",
];

/// How many tokens go by before the palette repeats. Eight is enough that a
/// repeat is never adjacent, and few enough that every colour stays legible.
pub const COLOURS: u32 = TOKEN_TYPES.len() as u32;

pub fn token_types() -> Vec<SemanticTokenType> {
    TOKEN_TYPES
        .iter()
        .copied()
        .map(SemanticTokenType::new)
        .collect()
}

pub fn token_modifiers() -> Vec<SemanticTokenModifier> {
    Vec::new()
}

/// Every token in `tree`, encoded for `textDocument/semanticTokens/full`.
pub fn tokens(tree: &Tree, index: &LineIndex) -> Vec<SemanticToken> {
    let mut flat = Vec::new();
    walk(tree, tree.root(), &mut flat);

    let mut encoded = Vec::new();
    let (mut previous_line, mut previous_start) = (0, 0);

    for (order, token) in flat.iter().enumerate() {
        let token_type = order as u32 % COLOURS;

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
                token_modifiers_bitset: 0,
            });
            (previous_line, previous_start) = (line, start);
        }
    }

    encoded
}

/// Tokens in source order.
///
/// Whitespace is left out — colouring it says nothing, since there is no glyph
/// to colour. Comments stay in, and get a colour like anything else.
fn walk(tree: &Tree, node: NodeId, out: &mut Vec<Token>) {
    for child in tree.children(node) {
        match child {
            Child::Node(child) => walk(tree, child, out),
            Child::Token(token) if token.kind != TokenKind::Whitespace => out.push(token),
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
        tokens(&parser::parse(source).tree, &LineIndex::new(source))
    }

    /// The palette cycles per token, so no two neighbours share a colour.
    #[test]
    fn adjacent_tokens_never_share_a_colour() {
        for source in [
            "a+b*c",
            "f(a,b)",
            "a.b.c",
            "(a)",
            "a as u64 + b as u64 * c as u64 - d",
        ] {
            let encoded = of(source);
            for pair in encoded.windows(2) {
                assert_ne!(
                    pair[0].token_type, pair[1].token_type,
                    "{source:?} has two same-coloured tokens in a row"
                );
            }
        }
    }

    /// And it is the token's position in the stream that picks the colour,
    /// not anything about the token itself — two `+`s in a row look different.
    #[test]
    fn colour_comes_from_order() {
        let types: Vec<_> = of("a+b+c").iter().map(|t| t.token_type).collect();
        assert_eq!(types, [0, 1, 2, 3, 4]);
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
