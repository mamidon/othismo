//! Byte offset ↔ LSP position.
//!
//! The front end deals in byte offsets into UTF-8 source. LSP deals in
//! line/character pairs, where "character" means whatever encoding was
//! negotiated. Under UTF-8 the column is a byte offset within the line and this
//! is only a line lookup; under UTF-16 it isn't, which is the reason to ask for
//! UTF-8 in the first place.

/// Start offset of every line, in bytes.
pub struct LineIndex {
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> LineIndex {
        let mut line_starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        LineIndex {
            line_starts,
            len: text.len(),
        }
    }

    /// Where `line` starts, in bytes. Past the last line this is the end of the
    /// text, so a caller walking lines never has to bound-check.
    pub fn line_start(&self, line: u32) -> usize {
        self.line_starts
            .get(line as usize)
            .copied()
            .unwrap_or(self.len)
    }

    /// Line and byte-column for a byte offset. Offsets past the end clamp to the
    /// end, because a diagnostic pointing just past the last character is
    /// ordinary — "expected `}`" at EOF, for instance.
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.len);
        // The last line start that is <= offset.
        let line = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        (line as u32, (offset - self.line_starts[line]) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_offsets_to_lines() {
        let index = LineIndex::new("ab\ncd\n\nx");
        assert_eq!(index.line_col(0), (0, 0));
        assert_eq!(index.line_col(2), (0, 2)); // the newline itself
        assert_eq!(index.line_col(3), (1, 0));
        assert_eq!(index.line_col(6), (2, 0)); // empty line
        assert_eq!(index.line_col(7), (3, 0));
    }

    #[test]
    fn clamps_past_end() {
        let index = LineIndex::new("ab");
        assert_eq!(index.line_col(99), (0, 2));
    }

    #[test]
    fn counts_bytes_not_characters() {
        // "é" is two bytes in UTF-8; under the UTF-8 encoding we negotiate, the
        // column after it is 2, not 1.
        let index = LineIndex::new("é!");
        assert_eq!(index.line_col(2), (0, 2));
    }
}
