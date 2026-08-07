//! The concrete syntax tree: what a node can be, and how the tree is stored.
//!
//! The tree is a flat `Vec<Event>` in depth-first preorder, not a graph of
//! pointers. A node is an `Open`, everything up to its matching `Close`, and
//! nothing else — so structure is carried by balanced brackets and a node is
//! addressed by a `u32`. That buys three things the language server wants:
//! one contiguous allocation for a whole file, spans derived from tokens
//! rather than stored and kept in sync, and trivia that is just more `Token`
//! events sitting between the significant ones.
//!
//! Nesting is an invariant of construction rather than something to validate:
//! a `Close` is only ever pushed by popping the builder's own stack, so a
//! child's `Open`/`Close` cannot straddle its parent's.
//!
//! # Two things a builder has to know
//!
//! **`Open::close` is patched in later.** A node's extent isn't known when it
//! opens, so `close` starts at [`Event::UNSET`] and every one of them is filled
//! by a single backward-free pass once parsing finishes. Nothing may read
//! `close` before that pass runs.
//!
//! **A node sometimes has to open *before* something already emitted.** The
//! parser doesn't learn that `a` was the left operand of a `BinaryExpr` until
//! it reaches the `+`, by which point `a`'s events are already in the vector.
//! The fix is to remember the index where the operand started and insert an
//! `Open` there. That is why `close` is patched in one pass at the end instead
//! of when each node closes — an insert would invalidate every index recorded
//! after it, and there is nothing to invalidate if nothing has been recorded
//! yet. The insert itself is a memmove of the operand's own events, which for
//! any expression a person would write is a handful.
//!
//! (rust-analyzer instead threads a `forward_parent` link between events and
//! reorders in the final pass, which is O(1) per wrap rather than O(n). It is
//! the right answer for a left-associative chain thousands of terms long, and
//! the wrong amount of machinery until one shows up.)

use tokenizer::{Span, Token};

/// One entry in the flat tree.
///
/// 16 bytes: the `Token` variant is the wide one, at a `TokenKind` plus a
/// `Span`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    /// Begins a node. `close` is the index of the matching [`Event::Close`],
    /// which makes "skip this whole subtree" an addition rather than a scan.
    Open { kind: NodeKind, close: u32 },
    /// A leaf. Trivia included, once the parser starts emitting it.
    Token(Token),
    /// Ends the innermost open node.
    Close,
}

impl Event {
    /// The `close` of a node that hasn't been closed and patched yet.
    pub const UNSET: u32 = u32::MAX;
}

/// Every node the grammar can produce.
///
/// Separate from [`TokenKind`] rather than merged into one `SyntaxKind` the
/// way rowan does it — [`Event`] already distinguishes a node from a leaf, so
/// merging would only add unreachable variants to both matches.
///
/// A missing child is *not* a kind here. Something that isn't in the source
/// occupies no tokens and so has no node; it becomes an error node during
/// lowering, when a slot goes unfilled. [`NodeKind::Error`] is the other case
/// — tokens that *are* present and didn't parse, which need a node because
/// every byte has to stay reachable from the tree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NodeKind {
    // ---- Root -------------------------------------------------------------
    /// A file is a block (§3): statements, then an optional trailing
    /// expression with no `;` that is the file's value.
    SourceFile,

    // ---- Statements -------------------------------------------------------
    /// `let mut? pattern (: Type)? = Expr ;` — the initializer is not
    /// optional (§3).
    LetStmt,
    /// `place assignOp Expr ;` (§3). The place is parsed as an ordinary
    /// expression and checked to be a name, field, or index later — a bad
    /// place deserves "you can't assign to a call", which needs the parse.
    AssignStmt,
    ExprStmt,
    WhileStmt,
    BreakStmt,
    ContinueStmt,
    /// `return ;` or `return Expr ;` (§4).
    ReturnStmt,

    // ---- Declarations -----------------------------------------------------
    // Statements too, per §3 collapsing Lox's declaration/statement split.
    // A leading `DocComment` token is a child of the declaration it attaches
    // to (§1); one attached to nothing stays a stray token in its enclosing
    // block, with a warning.
    FnDecl,
    ParamList,
    /// `name: Type` or `name: mut Type` — the `mut` is the parameter's, not
    /// the type's (§5), so it lives here.
    Param,
    /// `-> Type`. Also how [`NodeKind::FnType`] tells its return from its
    /// parameters.
    RetType,
    StructDecl,
    FieldDeclList,
    /// `name: Type` in a struct body (§6). Always annotated.
    FieldDecl,
    /// `type Name = Type ;` (§6).
    TypeAliasDecl,

    // ---- Expressions ------------------------------------------------------
    /// An int, float, string, char, `true`, or `false`. One kind, because the
    /// token underneath already says which, and the value is decoded from the
    /// span on demand by `tokenizer::literal_value`.
    LiteralExpr,
    NameExpr,
    /// `{ … }` — an expression (§2), so `if` and `while` bodies and function
    /// bodies are all the same node.
    BlockExpr,
    /// An expression (§2). With no `else` its type is unit.
    IfExpr,
    /// Grouping and nothing else (§2).
    ParenExpr,
    /// `()` — the unit value. Its own kind rather than a childless
    /// [`NodeKind::ParenExpr`], so lowering reads the kind instead of
    /// counting children.
    UnitExpr,
    /// Prefix `-`, `!`, `~`.
    UnaryExpr,
    BinaryExpr,
    /// `x as T` (§2) — explicit and trapping.
    CastExpr,
    CallExpr,
    ArgList,
    /// `a[i]`. In the precedence table at level 1 (§2); nothing but `Str` is
    /// indexable until §8 brings collections, which is the type checker's
    /// complaint to make, not the parser's.
    IndexExpr,
    /// `a.b`.
    FieldExpr,
    /// `a.b(…)`. Deliberately *not* a `CallExpr` wrapping a `FieldExpr`:
    /// that spelling would decide that `obj.method` is a field access
    /// yielding a callable, which is Lox's model and exactly the thing §2
    /// hands to §11 undecided.
    MethodCallExpr,
    /// `Point { x: 1, y: 2 }` (§6).
    StructLitExpr,
    FieldInitList,
    FieldInit,
    /// `|x| expr` (§5). Parameter types are optional here, unlike a `fn`.
    LambdaExpr,
    LambdaParamList,
    LambdaParam,

    // ---- Patterns ---------------------------------------------------------
    /// The whole of patterns today: a plain name (§3). §7 adds the rest, and
    /// a `let` whose first child is already a pattern node absorbs that
    /// without the shape changing.
    NamePat,

    // ---- Types ------------------------------------------------------------
    /// `u64`, `Str`, `Point`. Generic arguments arrive with §8.
    NameType,
    /// `fn(u64) -> u64` (§5). Parameter types are bare children; the return
    /// is a [`NodeKind::RetType`], which is what tells them apart.
    FnType,
    /// `()` (§6).
    UnitType,

    // ---- Recovery ---------------------------------------------------------
    /// Tokens that were present and didn't parse. Always accompanied by a
    /// diagnostic, and it exists so that skipped input stays in the tree
    /// rather than being dropped on the floor.
    Error,
}

/// A parsed file: the flat event vector, and nothing else.
pub struct Tree {
    events: Vec<Event>,
}

/// The index of an [`Event::Open`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub u32);

/// What sits directly inside a node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Child {
    Node(NodeId),
    Token(Token),
}

impl Tree {
    /// Built only by [`crate::builder::TreeBuilder`], which is what guarantees
    /// the brackets balance and every `close` is patched.
    pub(crate) fn from_events(events: Vec<Event>) -> Tree {
        Tree { events }
    }

    /// The outermost node. Always a [`NodeKind::SourceFile`], and always at
    /// index 0, because the parser opens it before reading anything.
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn kind(&self, node: NodeId) -> NodeKind {
        match self.events[node.0 as usize] {
            Event::Open { kind, .. } => kind,
            _ => unreachable!("a NodeId always points at an Open"),
        }
    }

    /// The node's extent, derived from the tokens it covers rather than
    /// stored — so there is nothing to keep in sync when the tree is edited.
    ///
    /// A node covering no tokens gets the empty span where it would have
    /// started, which is what a diagnostic wants to point at anyway.
    pub fn span(&self, node: NodeId) -> Span {
        let close = self.close(node);
        let mut tokens = self.events[node.0 as usize + 1..close]
            .iter()
            .filter_map(|event| match event {
                Event::Token(token) => Some(token.span),
                _ => None,
            });
        match tokens.next() {
            Some(first) => Span::new(
                first.start as usize,
                tokens.next_back().unwrap_or(first).end as usize,
            ),
            None => Span::empty_at(0),
        }
    }

    /// Direct children, in source order. A subtree is skipped by its `close`,
    /// so this visits each child once and never descends.
    pub fn children(&self, node: NodeId) -> impl Iterator<Item = Child> + '_ {
        let close = self.close(node);
        let mut index = node.0 as usize + 1;
        std::iter::from_fn(move || {
            if index >= close {
                return None;
            }
            match self.events[index] {
                Event::Open { close, .. } => {
                    let child = NodeId(index as u32);
                    index = close as usize + 1;
                    Some(Child::Node(child))
                }
                Event::Token(token) => {
                    index += 1;
                    Some(Child::Token(token))
                }
                Event::Close => None,
            }
        })
    }

    fn close(&self, node: NodeId) -> usize {
        match self.events[node.0 as usize] {
            Event::Open { close, .. } => {
                debug_assert_ne!(close, Event::UNSET, "tree was read before it was finished");
                close as usize
            }
            _ => unreachable!("a NodeId always points at an Open"),
        }
    }
}
