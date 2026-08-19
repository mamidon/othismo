//! The tree walk.
//!
//! Straight over the concrete syntax tree, with no lowering in between. That is
//! a deliberate stopgap: the IL is being designed separately and this will be
//! rewritten to consume it, so the cheapest thing that runs a program today is
//! worth more than a second tree that has to be kept in step with the first.
//!
//! Two consequences of walking the concrete tree, both visible below:
//!
//! * **Children are found by position, not by name.** A `LetStmt`'s pattern is
//!   its first child node and its initializer is its last; there is no
//!   `.initializer()` accessor to ask, because nothing has resolved the slots
//!   yet. Each such shape is spelled out where it's read, against the rule in
//!   `parser::stmt` that built it.
//! * **Trivia is in the tree.** Whitespace and comments are token children like
//!   any other, so anything looking for an operator token filters them out.
//!
//! # What this stage evaluates
//!
//! Expressions, `let`, assignment, blocks, `if`, and `while` — everything that
//! needs neither functions nor a type checker. `fn`, `struct`, `type`,
//! `return`, calls, lambdas, `as`, indexing, and field access all parse and
//! then report [`RuntimeErrorKind::Unsupported`], because "not implemented yet"
//! is a different thing to hear than "syntax error".

use parser::{Child, NodeId, NodeKind, Tree};
use tokenizer::{Literal, NumericType, Span, Token, TokenKind, literal_value};

use crate::env::{AssignError, Env};
use crate::error::{RuntimeError, RuntimeErrorKind};
use crate::ops;
use crate::value::Value;

/// How evaluation stops early.
///
/// `break` and `continue` are not errors, but they unwind exactly like one —
/// out of however many blocks are open, to the nearest `while`. Putting all
/// three in the error slot of a `Result` means every rule propagates them with
/// `?` and none of them has to say so.
///
/// The spans on `Break` and `Continue` are for the message when one reaches the
/// top with no loop to have applied to (§4).
enum Control {
    Error(RuntimeError),
    Break(Span),
    Continue(Span),
}

impl From<RuntimeError> for Control {
    fn from(error: RuntimeError) -> Control {
        Control::Error(error)
    }
}

type Eval<T> = Result<T, Control>;

pub(crate) struct Interpreter<'a> {
    tree: &'a Tree,
    source: &'a str,
    env: Env,
}

impl<'a> Interpreter<'a> {
    pub(crate) fn new(tree: &'a Tree, source: &'a str) -> Interpreter<'a> {
        Interpreter {
            tree,
            source,
            env: Env::new(),
        }
    }

    /// Evaluates the file and hands back its value.
    ///
    /// A file is a block (§3): statements, then an optional trailing expression
    /// with no `;` that is the file's value. A file that ends in a statement is
    /// worth `()`.
    pub(crate) fn run(mut self) -> Result<Value, RuntimeError> {
        let root = self.tree.root();
        if self.tree.kind(root) != NodeKind::SourceFile {
            return Err(RuntimeError::new(
                RuntimeErrorKind::MalformedProgram,
                self.span(root),
            ));
        }
        match self.body(root) {
            Ok(value) => Ok(value),
            Err(Control::Error(error)) => Err(error),
            // §4: `break` and `continue` apply to the innermost enclosing loop.
            // Getting here means there wasn't one.
            Err(Control::Break(span)) => {
                Err(RuntimeError::new(RuntimeErrorKind::BreakOutsideLoop, span))
            }
            Err(Control::Continue(span)) => Err(RuntimeError::new(
                RuntimeErrorKind::ContinueOutsideLoop,
                span,
            )),
        }
    }

    // ---- Blocks -----------------------------------------------------------

    /// A block's statements, in a scope of their own.
    ///
    /// The scope is popped on the way out whatever happened, including a
    /// `break` unwinding through — otherwise the loop it lands in would carry
    /// on with the body's scope still on the stack.
    fn block(&mut self, node: NodeId) -> Eval<Value> {
        self.env.push();
        let result = self.body(node);
        self.env.pop();
        result
    }

    /// The contents of a block, in whatever scope is already open. The file's
    /// own body uses this directly; every other block goes through
    /// [`Interpreter::block`].
    fn body(&mut self, node: NodeId) -> Eval<Value> {
        let mut value = Value::Unit;
        for child in self.child_nodes(node) {
            let kind = self.tree.kind(child);
            if is_statement(kind) {
                self.statement(child, kind)?;
                // §2: a `;` discards, so a block that ends in a statement is
                // worth unit however much its last expression produced.
                value = Value::Unit;
            } else {
                // The parser wraps every expression but a trailing one in an
                // `ExprStmt`, so a bare expression node is the block's value.
                value = self.expr(child)?;
            }
        }
        Ok(value)
    }

    // ---- Statements -------------------------------------------------------

    fn statement(&mut self, node: NodeId, kind: NodeKind) -> Eval<()> {
        match kind {
            NodeKind::LetStmt => self.let_stmt(node),
            NodeKind::AssignStmt => self.assign_stmt(node),
            NodeKind::ExprStmt => {
                // Evaluate and discard. §3 asks nothing of the value's type and
                // marks no discard as deliberate.
                let Some(&inner) = self.child_nodes(node).first() else {
                    return self.malformed(node);
                };
                self.expr(inner)?;
                Ok(())
            }
            NodeKind::WhileStmt => self.while_stmt(node),
            NodeKind::BreakStmt => Err(Control::Break(self.span(node))),
            NodeKind::ContinueStmt => Err(Control::Continue(self.span(node))),
            NodeKind::ReturnStmt => self.unsupported(node, "`return`"),
            NodeKind::FnDecl => self.unsupported(node, "a function declaration"),
            NodeKind::StructDecl => self.unsupported(node, "a struct declaration"),
            NodeKind::TypeAliasDecl => self.unsupported(node, "a type alias"),
            _ => unreachable!("is_statement admits no other kind"),
        }
    }

    /// `let mut? name (: Type)? = expr ;` (§3).
    ///
    /// The child nodes are the pattern, then the annotation if one was written,
    /// then the initializer — so the pattern is first and the initializer last.
    ///
    /// **The annotation is read and ignored.** There is no type checker yet, so
    /// there is nothing to check it against; `let x: u8 = 300;` binds `300`.
    /// That is a gap to close with §10, not a decision.
    fn let_stmt(&mut self, node: NodeId) -> Eval<()> {
        let nodes = self.child_nodes(node);
        if nodes.len() < 2 {
            // §3 requires an initializer, so the parser has already complained.
            return self.malformed(node);
        }
        let name = self.binding_name(nodes[0])?;
        let mutable = self.tokens(node).any(|token| token.kind == TokenKind::Mut);
        let value = self.expr(*nodes.last().expect("checked above"))?;
        self.env.declare(name, value, mutable);
        Ok(())
    }

    /// `place = expr ;` (§3).
    ///
    /// The place is parsed as an ordinary expression and checked here, which is
    /// what lets the message name what was assigned to.
    fn assign_stmt(&mut self, node: NodeId) -> Eval<()> {
        let nodes = self.child_nodes(node);
        let [place, value] = nodes[..] else {
            return self.malformed(node);
        };
        // §3's place is a name, a field, or an index. Fields and indexes need
        // §6's types, so a name is the only one that exists yet.
        if self.tree.kind(place) != NodeKind::NameExpr {
            return self.fail(place, RuntimeErrorKind::NotAPlace);
        }
        let name = self.binding_name(place)?;
        let value = self.expr(value)?;
        match self.env.assign(name, value) {
            Ok(()) => Ok(()),
            Err(AssignError::Unknown) => {
                self.fail(place, RuntimeErrorKind::UnknownName(name.to_string()))
            }
            Err(AssignError::Immutable) => {
                self.fail(place, RuntimeErrorKind::ImmutableBinding(name.to_string()))
            }
        }
    }

    /// `while c { … }` — the only loop, and a statement, so its value is unit
    /// (§4).
    fn while_stmt(&mut self, node: NodeId) -> Eval<()> {
        let nodes = self.child_nodes(node);
        let [condition, body] = nodes[..] else {
            return self.malformed(node);
        };
        loop {
            if !self.condition(condition)? {
                return Ok(());
            }
            match self.block(body) {
                Ok(_) => {}
                // §4: unlabelled, applying to the innermost enclosing loop —
                // which is this one, so neither travels any further.
                Err(Control::Break(_)) => return Ok(()),
                Err(Control::Continue(_)) => {}
                Err(other) => return Err(other),
            }
        }
    }

    // ---- Expressions ------------------------------------------------------

    fn expr(&mut self, node: NodeId) -> Eval<Value> {
        match self.tree.kind(node) {
            NodeKind::LiteralExpr => self.literal(node),
            NodeKind::NameExpr => {
                let name = self.binding_name(node)?;
                match self.env.get(name) {
                    Some(value) => Ok(value.clone()),
                    None => self.fail(node, RuntimeErrorKind::UnknownName(name.to_string())),
                }
            }
            // §2: grouping and nothing else — it does not change a value's
            // type, meaning, or evaluation.
            NodeKind::ParenExpr => match self.child_nodes(node).first() {
                Some(&inner) => self.expr(inner),
                None => self.malformed(node),
            },
            NodeKind::UnitExpr => Ok(Value::Unit),
            NodeKind::BlockExpr => self.block(node),
            NodeKind::IfExpr => self.if_expr(node),
            NodeKind::UnaryExpr => self.unary(node),
            NodeKind::BinaryExpr => self.binary(node),

            NodeKind::CastExpr => self.unsupported(node, "the `as` operator"),
            NodeKind::CallExpr | NodeKind::MethodCallExpr => self.unsupported(node, "a call"),
            NodeKind::IndexExpr => self.unsupported(node, "indexing"),
            NodeKind::FieldExpr => self.unsupported(node, "field access"),
            NodeKind::StructLitExpr => self.unsupported(node, "a struct literal"),
            NodeKind::LambdaExpr => self.unsupported(node, "a lambda"),

            // An `Error` node, or a node no expression position can hold.
            // Reachable only from a tree that didn't parse cleanly.
            _ => self.malformed(node),
        }
    }

    /// `if c { … } else if d { … } else { … }` — an expression (§2). With no
    /// `else` its value is unit, which is what makes it usable as a statement
    /// but not as a value.
    fn if_expr(&mut self, node: NodeId) -> Eval<Value> {
        let nodes = self.child_nodes(node);
        let (Some(&condition), Some(&consequence)) = (nodes.first(), nodes.get(1)) else {
            return self.malformed(node);
        };
        if self.condition(condition)? {
            return self.expr(consequence);
        }
        // `else if` is `else` followed by another `if` (§4), so the alternative
        // is a block or another `IfExpr` and `expr` handles both.
        match nodes.get(2) {
            Some(&alternative) => self.expr(alternative),
            None => Ok(Value::Unit),
        }
    }

    fn unary(&mut self, node: NodeId) -> Eval<Value> {
        let (Some(operator), Some(&operand)) =
            (self.tokens(node).next(), self.child_nodes(node).first())
        else {
            return self.malformed(node);
        };
        let operand = self.expr(operand)?;
        match ops::unary(operator.kind, operand) {
            Ok(value) => Ok(value),
            Err(kind) => self.fail(node, kind),
        }
    }

    fn binary(&mut self, node: NodeId) -> Eval<Value> {
        let nodes = self.child_nodes(node);
        // The operands are nodes and the operator is the one significant token
        // between them — the operands' own tokens are inside their subtrees.
        let [left, right] = nodes[..] else {
            return self.malformed(node);
        };
        let Some(operator) = self.tokens(node).next() else {
            return self.malformed(node);
        };

        // §2: `&&` and `||` short-circuit, which makes them control flow rather
        // than operators, so they decide what to evaluate rather than being
        // handed two values.
        if matches!(operator.kind, TokenKind::AmpAmp | TokenKind::PipePipe) {
            let short_circuits_on = operator.kind == TokenKind::PipePipe;
            if self.logical_operand(left, operator.kind)? == short_circuits_on {
                return Ok(Value::Bool(short_circuits_on));
            }
            let right = self.logical_operand(right, operator.kind)?;
            return Ok(Value::Bool(right));
        }

        // §2: left to right, everywhere, specified — so that an interpreter and
        // a wasm compiler cannot disagree about it.
        let left = self.expr(left)?;
        let right = self.expr(right)?;
        match ops::binary(operator.kind, left, right) {
            Ok(value) => Ok(value),
            Err(kind) => self.fail(node, kind),
        }
    }

    /// An operand of `&&` or `||`, which §2 defines on `bool` only.
    fn logical_operand(&mut self, node: NodeId, operator: TokenKind) -> Eval<bool> {
        let value = self.expr(node)?;
        match value {
            Value::Bool(value) => Ok(value),
            _ => self.fail(
                node,
                RuntimeErrorKind::UnaryTypeMismatch {
                    operator: operator.spelling().expect("`&&` and `||` are spelled"),
                    operand: value.type_name(),
                },
            ),
        }
    }

    /// The condition of an `if` or a `while`.
    ///
    /// §2: there is no truthiness. A condition is a `bool` or it is an error —
    /// which falls out of §1 having no `nil` and no implicit conversion.
    fn condition(&mut self, node: NodeId) -> Eval<bool> {
        let value = self.expr(node)?;
        match value {
            Value::Bool(value) => Ok(value),
            _ => self.fail(node, RuntimeErrorKind::ConditionNotBool(value.type_name())),
        }
    }

    /// The value a literal token names, decoded from its span on demand.
    fn literal(&mut self, node: NodeId) -> Eval<Value> {
        let Some(token) = self.tokens(node).next() else {
            return self.malformed(node);
        };
        // `None` means the tokenizer already reported why, and there is nothing
        // to add.
        let Some(literal) = literal_value(token, self.source) else {
            return self.malformed(node);
        };
        match literal {
            // §1 makes an unsuffixed literal an unpinned constant that acquires
            // a type from context, and pinning is §10's job. So the suffix is
            // read for one thing only: whether this is an integer or a float.
            Literal::Int { value, suffix } => {
                if matches!(suffix, Some(NumericType::F32 | NumericType::F64)) {
                    return Ok(Value::Float(value as f64));
                }
                match i64::try_from(value) {
                    Ok(value) => Ok(Value::Int(value)),
                    Err(_) => self.fail(node, RuntimeErrorKind::IntegerTooLarge),
                }
            }
            Literal::Float { value, .. } => Ok(Value::Float(value)),
            Literal::Str(text) => Ok(Value::string(&text)),
            Literal::Char(character) => Ok(Value::Char(character)),
            Literal::Bool(value) => Ok(Value::Bool(value)),
        }
    }

    // ---- Reading the tree -------------------------------------------------

    /// The name a `NamePat` or a `NameExpr` holds.
    ///
    /// Borrowed from the source rather than copied: a `Token` is a kind and a
    /// span, so this is a slice, and only `declare` and the error paths need an
    /// owned `String`.
    fn binding_name(&self, node: NodeId) -> Eval<&'a str> {
        match self.tokens(node).next() {
            Some(token) if token.kind == TokenKind::Ident => Ok(token.span.text(self.source)),
            _ => self.fail(node, RuntimeErrorKind::MalformedProgram),
        }
    }

    /// A node's extent, with the trivia in front of it left out.
    ///
    /// [`Tree::span`] covers every token a node holds, and the parser attaches
    /// a comment or a run of whitespace to the node whose first token follows
    /// it — so a node's stored extent can begin a blank line above the thing it
    /// names. Fine for a formatter, wrong for a message with a caret under it,
    /// which wants the first token a person actually wrote.
    ///
    /// A subtree walk, on the error path only.
    fn span(&self, node: NodeId) -> Span {
        match (self.edge(node, false), self.edge(node, true)) {
            (Some(first), Some(last)) => {
                Span::new(first.span.start as usize, last.span.end as usize)
            }
            // A node covering nothing but trivia, or nothing at all.
            _ => self.tree.span(node),
        }
    }

    /// The first or last significant token anywhere under `node`.
    fn edge(&self, node: NodeId, last: bool) -> Option<Token> {
        let mut children: Vec<_> = self.tree.children(node).collect();
        if last {
            children.reverse();
        }
        children.into_iter().find_map(|child| match child {
            Child::Token(token) if !token.is_trivia() => Some(token),
            Child::Node(child) => self.edge(child, last),
            Child::Token(_) => None,
        })
    }

    fn child_nodes(&self, node: NodeId) -> Vec<NodeId> {
        self.tree
            .children(node)
            .filter_map(|child| match child {
                Child::Node(node) => Some(node),
                Child::Token(_) => None,
            })
            .collect()
    }

    /// A node's own token children, trivia dropped. Trivia is in the tree
    /// because the tree is lossless, and no rule here has any use for it.
    fn tokens(&self, node: NodeId) -> impl Iterator<Item = Token> + '_ {
        self.tree.children(node).filter_map(|child| match child {
            Child::Token(token) if !token.is_trivia() => Some(token),
            _ => None,
        })
    }

    // ---- Failing ----------------------------------------------------------

    fn fail<T>(&self, node: NodeId, kind: RuntimeErrorKind) -> Eval<T> {
        Err(Control::Error(RuntimeError::new(kind, self.span(node))))
    }

    fn unsupported<T>(&self, node: NodeId, what: &'static str) -> Eval<T> {
        self.fail(node, RuntimeErrorKind::Unsupported(what))
    }

    fn malformed<T>(&self, node: NodeId) -> Eval<T> {
        self.fail(node, RuntimeErrorKind::MalformedProgram)
    }
}

/// Whether a node is a statement rather than an expression.
///
/// §3 collapses Lox's declaration/statement split, so `fn`, `struct`, and
/// `type` are in here with the rest.
fn is_statement(kind: NodeKind) -> bool {
    use NodeKind::*;
    matches!(
        kind,
        LetStmt
            | AssignStmt
            | ExprStmt
            | WhileStmt
            | BreakStmt
            | ContinueStmt
            | ReturnStmt
            | FnDecl
            | StructDecl
            | TypeAliasDecl
    )
}
