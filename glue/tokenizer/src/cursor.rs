//! A position in a slice, with lookahead and lookbehind.
//!
//! Generic over the item type because the parser wants exactly this shape over
//! `Token` — `peek(n)`, `try_consume(|t| t.kind == Semicolon)`, and `trail(0)`
//! for "expected `;` after *this*" diagnostics. One data structure, two
//! instantiations.
//!
//! The tokenizer instantiates it over `u8`, so `index()` *is* a byte offset and
//! `peek`/`trail` are slice indexing. Scanning bytes is safe for a language
//! whose identifiers are ASCII (§1): every UTF-8 continuation byte is ≥ 0x80,
//! so it can never equal `"` or `*` or `/`, and any position the lexer stops at
//! is therefore a character boundary. The two places a `char` is genuinely
//! observable — a character literal, and an error pointing at a stray `é` — use
//! [`Cursor::consume_utf8`].

#[derive(Clone, Copy)]
pub struct Cursor<'a, T> {
    items: &'a [T],
    index: usize,
}

impl<'a, T: Copy> Cursor<'a, T> {
    pub fn new(items: &'a [T]) -> Cursor<'a, T> {
        Cursor { items, index: 0 }
    }

    /// How many items have been consumed. Over `u8`, this is a byte offset.
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn at_end(&self) -> bool {
        self.index >= self.items.len()
    }

    /// The `n`th item not yet consumed. `peek(0)` is the next one.
    pub fn peek(&self, n: usize) -> Option<T> {
        self.items.get(self.index + n).copied()
    }

    /// The `n`th item already consumed, counting backwards. `trail(0)` is the
    /// most recent one.
    pub fn trail(&self, n: usize) -> Option<T> {
        self.index
            .checked_sub(n + 1)
            .and_then(|i| self.items.get(i).copied())
    }

    pub fn consume(&mut self) -> Option<T> {
        let item = self.peek(0)?;
        self.index += 1;
        Some(item)
    }

    /// Consume the next item only if it satisfies `predicate`.
    pub fn try_consume(&mut self, predicate: impl FnOnce(T) -> bool) -> Option<T> {
        match self.peek(0) {
            Some(item) if predicate(item) => {
                self.index += 1;
                Some(item)
            }
            _ => None,
        }
    }

    /// Consume items while they satisfy `predicate`. Returns how many.
    pub fn consume_while(&mut self, predicate: impl Fn(T) -> bool) -> usize {
        let start = self.index;
        while self.try_consume(&predicate).is_some() {}
        self.index - start
    }

    /// Everything not yet consumed.
    pub fn rest(&self) -> &'a [T] {
        &self.items[self.index.min(self.items.len())..]
    }
}

impl<'a, T: Copy + PartialEq> Cursor<'a, T> {
    /// Consume the next item if it equals `want`.
    pub fn eat(&mut self, want: T) -> bool {
        self.try_consume(|item| item == want).is_some()
    }

    /// Consume `want` in full, or nothing at all. This is what makes `"""`,
    /// `...`, and `*/` one line each.
    pub fn eat_seq(&mut self, want: &[T]) -> bool {
        if self.rest().starts_with(want) {
            self.index += want.len();
            true
        } else {
            false
        }
    }
}

impl<'a> Cursor<'a, u8> {
    /// Consume one whole UTF-8 character's worth of bytes, and return how many.
    ///
    /// The byte instantiation needs this in the handful of places where a
    /// character boundary is observable; everywhere else, byte-at-a-time is
    /// both correct and faster.
    pub fn consume_utf8(&mut self) -> usize {
        let Some(byte) = self.peek(0) else { return 0 };
        let width = match byte {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            // Not reachable from a `&str`, but the lexer must always advance.
            _ => 1,
        };
        let width = width.min(self.items.len() - self.index);
        self.index += width;
        width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peeks_and_trails() {
        let mut cursor = Cursor::new(b"abcd");
        assert_eq!(cursor.peek(0), Some(b'a'));
        assert_eq!(cursor.peek(2), Some(b'c'));
        assert_eq!(cursor.peek(9), None);
        assert_eq!(cursor.trail(0), None);

        cursor.consume();
        cursor.consume();
        assert_eq!(cursor.index(), 2);
        assert_eq!(cursor.trail(0), Some(b'b'));
        assert_eq!(cursor.trail(1), Some(b'a'));
        assert_eq!(cursor.trail(2), None);
    }

    #[test]
    fn try_consume_does_not_advance_on_failure() {
        let mut cursor = Cursor::new(b"ab");
        assert_eq!(cursor.try_consume(|b| b == b'z'), None);
        assert_eq!(cursor.index(), 0);
        assert_eq!(cursor.try_consume(|b| b == b'a'), Some(b'a'));
        assert_eq!(cursor.index(), 1);
    }

    #[test]
    fn eat_seq_is_all_or_nothing() {
        let mut cursor = Cursor::new(b"\"\"\"x");
        assert!(!cursor.eat_seq(b"\"\"\"\""));
        assert_eq!(cursor.index(), 0);
        assert!(cursor.eat_seq(b"\"\"\""));
        assert_eq!(cursor.index(), 3);
    }

    #[test]
    fn consume_utf8_advances_by_character() {
        let text = "aé中\u{1F600}";
        let mut cursor = Cursor::new(text.as_bytes());
        assert_eq!(cursor.consume_utf8(), 1);
        assert_eq!(cursor.consume_utf8(), 2);
        assert_eq!(cursor.consume_utf8(), 3);
        assert_eq!(cursor.consume_utf8(), 4);
        assert!(cursor.at_end());
    }

    #[test]
    fn works_over_other_item_types() {
        // The reason for the type parameter: the parser gets the same thing
        // over tokens.
        let mut cursor = Cursor::new(&[1u32, 2, 3][..]);
        assert_eq!(cursor.consume_while(|n| n < 3), 2);
        assert_eq!(cursor.peek(0), Some(3));
    }
}
