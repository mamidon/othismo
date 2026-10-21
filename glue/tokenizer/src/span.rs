//! Byte ranges into the source text.

use std::fmt;
use std::ops::Range;

/// A half-open byte range `[start, end)` into the source.
///
/// Offsets are bytes into UTF-8 source, not characters — which is what the
/// language server negotiates for, so a span is a column directly rather than
/// something to convert on every diagnostic.
///
/// `u32` rather than `usize`: a 4 GiB source file is not a case worth carrying
/// eight bytes per token for, and a `Token` that fits in a register pair is
/// worth more than the headroom.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        debug_assert!(start <= end, "span starts after it ends");
        Span {
            start: start as u32,
            end: end as u32,
        }
    }

    /// The empty span at `offset`. Used for `Eof`, and for any diagnostic that
    /// points *between* two characters rather than at one.
    pub fn empty_at(offset: usize) -> Span {
        Span::new(offset, offset)
    }

    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn range(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The source text this span covers.
    ///
    /// Never panics on well-formed input: the lexer only ever stops at a UTF-8
    /// character boundary, because every byte it compares against is ASCII and
    /// no continuation byte can equal one.
    pub fn text<'src>(&self, source: &'src str) -> &'src str {
        &source[self.range()]
    }

    pub fn contains(&self, offset: usize) -> bool {
        self.range().contains(&offset)
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

impl From<Span> for Range<usize> {
    fn from(span: Span) -> Range<usize> {
        span.range()
    }
}
