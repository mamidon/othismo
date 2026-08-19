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
//! Expressions, `let`, assignment, blocks, `if`, `while`, and §5's functions —
//! declarations, calls, lambdas, and `return`. `struct`, `type`, `as`,
//! indexing, field access, and method calls all parse and then report
//! [`RuntimeErrorKind::Unsupported`], because "not implemented yet" is a
//! different thing to hear than "syntax error".

use std::rc::Rc;

use parser::{Child, NodeId, NodeKind, Tree};
use tokenizer::{Literal, NumericType, Span, Token, TokenKind, literal_value};

use crate::env::{AssignError, Env, LookupError};
use crate::error::{RuntimeError, RuntimeErrorKind};
use crate::function::{Function, Param};
use crate::ops;
use crate::value::Value;

/// How deep a call chain may go.
///
/// §5: there is no tail-call guarantee, so deep recursion exhausts the stack
/// and traps. A real stack overflow aborts the process without a message, which
/// is the one outcome worth ruling out — so the trap is raised at a depth the
/// host stack still has plenty of room for. The number is a budget, not a
/// language rule; §15 owns resource limits.
const RECURSION_LIMIT: usize = 256;

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
    /// §4: `return` exits the enclosing *function*, not the block — so it
    /// travels through blocks and loops alike and is caught at a call.
    Return(Value, Span),
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
    /// How many calls are open, against [`RECURSION_LIMIT`].
    depth: usize,
}

impl<'a> Interpreter<'a> {
    pub(crate) fn new(tree: &'a Tree, source: &'a str) -> Interpreter<'a> {
        Interpreter {
            tree,
            source,
            env: Env::new(),
            depth: 0,
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
            // §4: `return` is for early exit from a function. A file is a
            // block, not a function — its value is its trailing expression.
            Err(Control::Return(_, span)) => Err(RuntimeError::new(
                RuntimeErrorKind::ReturnOutsideFunction,
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
        self.hoist(node)?;
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

    /// Declares every `fn` in a block before any of its statements run.
    ///
    /// §5 needs this: "mutual recursion needs top-level declarations to be
    /// order-independent", and §12 — which owes the general answer — is still
    /// empty. Hoisting per block rather than per file is the smallest rule that
    /// covers it, and it costs nothing, because §5 also says a `fn` captures
    /// nothing: there is no environment whose contents at the point of
    /// declaration could have differed.
    fn hoist(&mut self, node: NodeId) -> Eval<()> {
        for child in self.child_nodes(node) {
            if self.tree.kind(child) != NodeKind::FnDecl {
                continue;
            }
            let function = self.fn_decl(child)?;
            let name = function.name.clone().expect("a `fn` declaration is named");
            // Not mutable: §3's `mut` is for bindings a program assigns to, and
            // §5 has one name for one function.
            self.env
                .declare(&name, Value::Function(Rc::new(function)), false);
        }
        Ok(())
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
            NodeKind::ReturnStmt => self.return_stmt(node),
            // Already declared, by `hoist`, before this pass began.
            NodeKind::FnDecl => Ok(()),
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
            Err(problem) => self.fail(place, assign_failed(problem, name)),
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

    /// `return ;` or `return expr ;` — for *early* exit (§4). A well-shaped
    /// function often has none, since a body is a block and ends in its value.
    fn return_stmt(&mut self, node: NodeId) -> Eval<()> {
        let value = match self.child_nodes(node).first() {
            Some(&inner) => self.expr(inner)?,
            // §5: a function with no `-> T` returns unit — a real value with one
            // inhabitant, not an absence.
            None => Value::Unit,
        };
        Err(Control::Return(value, self.span(node)))
    }

    // ---- Expressions ------------------------------------------------------

    fn expr(&mut self, node: NodeId) -> Eval<Value> {
        match self.tree.kind(node) {
            NodeKind::LiteralExpr => self.literal(node),
            NodeKind::NameExpr => {
                let name = self.binding_name(node)?;
                match self.env.get(name) {
                    Ok(value) => Ok(value),
                    Err(problem) => self.fail(node, lookup_failed(problem, name)),
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

            NodeKind::CallExpr => self.call(node),
            NodeKind::LambdaExpr => self.lambda(node),

            NodeKind::CastExpr => self.unsupported(node, "the `as` operator"),
            // §11's, along with everything else about receivers.
            NodeKind::MethodCallExpr => self.unsupported(node, "a method call"),
            NodeKind::IndexExpr => self.unsupported(node, "indexing"),
            NodeKind::FieldExpr => self.unsupported(node, "field access"),
            NodeKind::StructLitExpr => self.unsupported(node, "a struct literal"),

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

    // ---- Functions (§5) ---------------------------------------------------

    /// `fn name(a: T, b: mut U) -> R { … }`.
    ///
    /// The annotations are read past and dropped. §5 requires them so that a
    /// function's meaning is readable without reading its body and so that §10
    /// can infer *inside* one — neither of which the interpreter can act on
    /// while there is no type checker to act with.
    fn fn_decl(&mut self, node: NodeId) -> Eval<Function> {
        let Some(name) = self
            .tokens(node)
            .find(|token| token.kind == TokenKind::Ident)
        else {
            return self.malformed(node);
        };
        let nodes = self.child_nodes(node);
        // The parameter list is always the first child node and the body always
        // the last; a `-> T` sits between them when it was written.
        let (Some(&params), Some(&body)) = (nodes.first(), nodes.last()) else {
            return self.malformed(node);
        };
        if nodes.len() < 2
            || self.tree.kind(params) != NodeKind::ParamList
            || self.tree.kind(body) != NodeKind::BlockExpr
        {
            return self.malformed(node);
        }

        let params = self.params(params, NodeKind::Param)?;
        let (captured, _) = self.env.capture();
        Ok(Function {
            name: Some(Rc::from(name.span.text(self.source))),
            params,
            body,
            // §5: a `fn` captures nothing — so the barrier covers every frame it
            // was declared under, leaving other functions reachable and nothing
            // else. See `crate::env`.
            barrier: captured.len(),
            captured,
        })
    }

    /// `(x) -> x + 1` (§5). Parameter types are optional and equally ignored: a
    /// `fn` is a declaration others read, a lambda is an argument read in place.
    fn lambda(&mut self, node: NodeId) -> Eval<Value> {
        let nodes = self.child_nodes(node);
        let (Some(&params), Some(&body)) = (nodes.first(), nodes.last()) else {
            return self.malformed(node);
        };
        if nodes.len() < 2 || self.tree.kind(params) != NodeKind::LambdaParamList {
            return self.malformed(node);
        }

        let params = self.params(params, NodeKind::LambdaParam)?;
        // §5: captures by reference, implicitly, with no capture list — which is
        // what holding the frames themselves gets. The barrier is inherited
        // rather than raised, so a lambda written inside a `fn` cannot reach
        // what the `fn` couldn't.
        let (captured, barrier) = self.env.capture();
        Ok(Value::Function(Rc::new(Function {
            name: None,
            params,
            body,
            captured,
            barrier,
        })))
    }

    fn params(&self, list: NodeId, kind: NodeKind) -> Eval<Vec<Param>> {
        let mut params = Vec::new();
        for child in self.child_nodes(list) {
            if self.tree.kind(child) != kind {
                return self.malformed(child);
            }
            let Some(name) = self
                .tokens(child)
                .find(|token| token.kind == TokenKind::Ident)
            else {
                return self.malformed(child);
            };
            params.push(Param {
                name: Rc::from(name.span.text(self.source)),
                // §5: parameters are immutable bindings by default, exactly like
                // a `let`. `mut` belongs to the parameter rather than its type,
                // which is why the token is here and not in the type node.
                mutable: self.tokens(child).any(|token| token.kind == TokenKind::Mut),
            });
        }
        Ok(params)
    }

    /// `f(a, b)`.
    ///
    /// §5 checks arity statically and there is nothing static here yet, so the
    /// count is checked at the call. That is a stand-in for a compile error and
    /// should stop being reachable when §10 lands.
    fn call(&mut self, node: NodeId) -> Eval<Value> {
        let nodes = self.child_nodes(node);
        let [callee, arguments] = nodes[..] else {
            return self.malformed(node);
        };
        if self.tree.kind(arguments) != NodeKind::ArgList {
            return self.malformed(node);
        }

        let function = match self.expr(callee)? {
            Value::Function(function) => function,
            other => {
                return self.fail(callee, RuntimeErrorKind::NotCallable(other.type_name()));
            }
        };

        let arguments = self.child_nodes(arguments);
        if arguments.len() != function.params.len() {
            return self.fail(
                node,
                RuntimeErrorKind::WrongArity {
                    callee: function.describe(),
                    expected: function.params.len(),
                    found: arguments.len(),
                },
            );
        }

        // §2: arguments evaluate left to right, before the call.
        let mut values = Vec::with_capacity(arguments.len());
        let mut writeback = Vec::new();
        for (param, &argument) in function.params.iter().zip(&arguments) {
            if param.mutable {
                writeback.push((self.mut_argument(argument, param)?, Rc::clone(&param.name)));
            }
            values.push(self.expr(argument)?);
        }

        self.invoke(&function, values, writeback, node)
    }

    /// Checks the argument for a `mut` parameter and names the binding it will
    /// be written back to.
    ///
    /// §5: `mut` means the function may mutate the caller's value, and the
    /// caller must pass a `mut` binding — so that a call which mutates is
    /// visible where it happens rather than only where it was declared.
    fn mut_argument(&mut self, argument: NodeId, param: &Param) -> Eval<&'a str> {
        let Some(name) = self.place_name(argument) else {
            return self.fail(
                argument,
                RuntimeErrorKind::MutArgumentNotAPlace {
                    parameter: param.name.to_string(),
                },
            );
        };
        match self.env.lookup(name) {
            Ok((_, true)) => Ok(name),
            Ok((_, false)) => self.fail(
                argument,
                RuntimeErrorKind::MutArgumentNotMutable {
                    parameter: param.name.to_string(),
                    argument: name.to_string(),
                },
            ),
            Err(problem) => self.fail(argument, lookup_failed(problem, name)),
        }
    }

    /// Runs a function body against its own environment.
    fn invoke(
        &mut self,
        function: &Function,
        arguments: Vec<Value>,
        writeback: Vec<(&'a str, Rc<str>)>,
        at: NodeId,
    ) -> Eval<Value> {
        if self.depth >= RECURSION_LIMIT {
            return self.fail(at, RuntimeErrorKind::RecursionLimit);
        }

        let saved = self.env.enter(&function.captured, function.barrier);
        for (param, value) in function.params.iter().zip(arguments) {
            self.env.declare(&param.name, value, param.mutable);
        }

        self.depth += 1;
        // §5: the body is a block, so its value is its trailing expression.
        let result = match self.expr(function.body) {
            Err(Control::Return(value, _)) => Ok(value),
            // §4's innermost enclosing loop is one in *this* function, and there
            // isn't one — a call is not something `break` reaches across.
            Err(Control::Break(span)) => Err(Control::Error(RuntimeError::new(
                RuntimeErrorKind::BreakOutsideLoop,
                span,
            ))),
            Err(Control::Continue(span)) => Err(Control::Error(RuntimeError::new(
                RuntimeErrorKind::ContinueOutsideLoop,
                span,
            ))),
            other => other,
        };
        self.depth -= 1;

        // A `mut` parameter's final value, read while its frame is still here.
        // Copy-in/copy-out rather than by reference — §5 leaves the choice to
        // §6, and the two are indistinguishable until there is an aggregate to
        // alias.
        let mut written = Vec::new();
        if result.is_ok() {
            for (place, param) in writeback {
                if let Ok(value) = self.env.get(&param) {
                    written.push((place, value));
                }
            }
        }
        self.env.leave(saved);

        let value = result?;
        for (place, value) in written {
            if let Err(problem) = self.env.assign(place, value) {
                return self.fail(at, assign_failed(problem, place));
            }
        }
        Ok(value)
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

    /// The name this expression is a place for, if it is one. §3's places are a
    /// name, a field, or an index; the other two need §6's types.
    fn place_name(&self, node: NodeId) -> Option<&'a str> {
        if self.tree.kind(node) != NodeKind::NameExpr {
            return None;
        }
        self.tokens(node)
            .next()
            .filter(|token| token.kind == TokenKind::Ident)
            .map(|token| token.span.text(self.source))
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

fn lookup_failed(problem: LookupError, name: &str) -> RuntimeErrorKind {
    match problem {
        LookupError::Unknown => RuntimeErrorKind::UnknownName(name.to_string()),
        LookupError::NotCaptured => RuntimeErrorKind::NotCaptured(name.to_string()),
    }
}

fn assign_failed(problem: AssignError, name: &str) -> RuntimeErrorKind {
    match problem {
        AssignError::Unknown => RuntimeErrorKind::UnknownName(name.to_string()),
        AssignError::NotCaptured => RuntimeErrorKind::NotCaptured(name.to_string()),
        AssignError::Immutable => RuntimeErrorKind::ImmutableBinding(name.to_string()),
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
