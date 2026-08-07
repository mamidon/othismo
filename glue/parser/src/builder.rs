//! Building the event vector.
//!
//! The parser only ever appends, with one exception — [`TreeBuilder::open_before`],
//! which is how a node gets to start earlier than the parser knew it would. See
//! the [`crate::syntax`] module docs for why that's an insert rather than a
//! link-and-reorder.
//!
//! A node is in one of two states, and each has its own type, so the state is
//! checked rather than remembered:
//!
//! ```ignore
//! let mut lhs = operand(builder);                     // Closed
//! while let Some(op) = p.peek_binary_operator() {
//!     let node = builder.open_before(lhs, BinaryExpr); // Closed -> Mark
//!     builder.token(op);
//!     operand(builder);
//!     lhs = builder.close(node);                       // Mark -> Closed
//! }
//! ```

use tokenizer::Token;

use crate::syntax::{Event, NodeKind, Tree};

/// A node that has been opened and not yet closed.
///
/// Neither `Copy` nor `Clone`: [`TreeBuilder::close`] consumes it, so a node
/// cannot be closed twice, and [`TreeBuilder::open_before`] cannot leave a
/// second mark behind pointing at a node that moved.
#[derive(Debug)]
#[must_use = "an opened node must be closed"]
pub struct Mark(u32);

/// A node that has been closed. Holding one is how the parser says "wrap what
/// I just built" once it learns that it should.
///
/// Dropping one is ordinary — most nodes are never wrapped.
#[derive(Debug)]
pub struct Closed(u32);

#[derive(Default)]
pub struct TreeBuilder {
    events: Vec<Event>,
    /// Indices of opened-and-not-yet-closed nodes, outermost first. Ascending
    /// by construction, which is what lets `open_before` know it isn't
    /// invalidating anything still open.
    open: Vec<u32>,
}

impl TreeBuilder {
    pub fn new() -> TreeBuilder {
        TreeBuilder::default()
    }

    /// Begins a node at the current position.
    pub fn open(&mut self, kind: NodeKind) -> Mark {
        let at = self.events.len() as u32;
        self.events.push(Event::Open {
            kind,
            close: Event::UNSET,
        });
        self.open.push(at);
        Mark(at)
    }

    /// Begins a node where `closed` begins, so that it encloses it.
    ///
    /// This is how a left-associative operator is parsed: the left operand is
    /// already in the vector by the time the operator is read, and the node
    /// that owns both has to start in front of it.
    ///
    /// The insert shifts every event at or after `closed`, so any *other*
    /// [`Closed`] at or after it is stale afterwards. Consuming `closed`
    /// covers the operand itself; the remaining rule is that the parser holds
    /// at most one `Closed` at a time, which is how an operand-then-operator
    /// loop is shaped anyway. Everything still *open* is checked below,
    /// because that is the case a nested parse could get wrong silently.
    pub fn open_before(&mut self, closed: Closed, kind: NodeKind) -> Mark {
        debug_assert!(
            self.open.last().is_none_or(|&open| open < closed.0),
            "open_before would move a node that is still open"
        );
        self.events.insert(
            closed.0 as usize,
            Event::Open {
                kind,
                close: Event::UNSET,
            },
        );
        self.open.push(closed.0);
        Mark(closed.0)
    }

    /// Adds a leaf. Every token the lexer produced goes through here, trivia
    /// included, or the tree stops being lossless.
    pub fn token(&mut self, token: Token) {
        self.events.push(Event::Token(token));
    }

    pub fn close(&mut self, mark: Mark) -> Closed {
        let opened = self.open.pop();
        debug_assert_eq!(opened, Some(mark.0), "closed a node that wasn't innermost");
        self.events.push(Event::Close);
        Closed(mark.0)
    }

    /// Patches every `close` and hands back the finished tree.
    ///
    /// One forward pass with a stack. The `close` fields cannot be filled in
    /// as each node closes, because an [`TreeBuilder::open_before`] later on
    /// would shift the very indices already written.
    pub fn finish(mut self) -> Tree {
        debug_assert!(
            self.open.is_empty(),
            "{} node(s) left open — the parser must close what it opens",
            self.open.len()
        );
        // Totality outranks the assertion: a builder bug should still yield a
        // well-formed tree in release rather than one that reads off the end.
        for _ in 0..self.open.len() {
            self.events.push(Event::Close);
        }
        self.open.clear();

        let mut stack: Vec<usize> = Vec::new();
        for index in 0..self.events.len() {
            match self.events[index] {
                Event::Open { .. } => stack.push(index),
                Event::Close => {
                    // Balanced by construction — `close` pops the same stack.
                    if let Some(opened) = stack.pop()
                        && let Event::Open { close, .. } = &mut self.events[opened]
                    {
                        *close = index as u32;
                    }
                }
                Event::Token(_) => {}
            }
        }

        Tree::from_events(self.events)
    }
}
