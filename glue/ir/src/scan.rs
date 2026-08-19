//! Two syntactic pre-passes, run before a block's statements are lowered.
//!
//! Both answer a question about a binding that can only be settled by looking
//! *forward*, at code not yet lowered, and both are deliberately syntactic. §1
//! makes that a requirement rather than a convenience: "a rule needing type
//! inference or dataflow to answer is a rule the two back ends will eventually
//! disagree about."
//!
//! # Is a binding ever assigned?
//!
//! §1 keeps an unpinned integer constant unpinned when its initializer is a
//! constant expression *and* it is never the target of an assignment in its
//! scope, so `let n = 3; n - 5` is `-2` rather than an underflow. Scanning a
//! subtree for assignments to a name is exactly the cheap syntactic test §1
//! asks for.
//!
//! # Is a binding captured?
//!
//! A captured binding that is also assigned needs a heap cell rather than a
//! slot, because a slot dies with its frame while §5 promises captured bindings
//! outlive it and that everyone holding one sees the same writes. That decision
//! has to be made where the binding is *created*, which is before the lambda
//! capturing it has been seen.
//!
//! # What they over-approximate
//!
//! [`BlockFacts`] ignores shadowing: it collects names, not bindings, so an
//! inner `let x` marks an outer `x` as assigned. The cost is a cell nobody
//! needed, never a missing one. [`free_vars`] does track scope, because there
//! imprecision costs a capture that cannot be resolved rather than a wasted
//! word.

use std::collections::HashSet;

use parser::{NodeId, NodeKind, Tree};

use crate::cst;

/// What a block's subtree says about the names declared in it.
#[derive(Default)]
pub struct BlockFacts {
    /// Names appearing as the target of an assignment (§1, §3).
    pub assigned: HashSet<String>,
    /// Names mentioned inside some lambda in this subtree (§5).
    pub captured: HashSet<String>,
}

impl BlockFacts {
    /// Whether a binding of this name, declared in this block, needs a cell.
    ///
    /// Both halves are necessary. A binding that is never assigned can be
    /// copied into the closure environment, because every copy stays equal
    /// forever — and under §6's reference semantics copying a struct binding
    /// copies the reference, so mutation of the *object* is visible either way.
    /// What a cell protects is assignment to the **name**.
    pub fn needs_cell(&self, name: &str) -> bool {
        self.captured.contains(name) && self.assigned.contains(name)
    }
}

pub fn block_facts(tree: &Tree, source: &str, block: NodeId) -> BlockFacts {
    let mut facts = BlockFacts::default();
    collect(tree, source, block, false, &mut facts);
    facts
}

fn collect(tree: &Tree, source: &str, node: NodeId, in_lambda: bool, facts: &mut BlockFacts) {
    let kind = tree.kind(node);

    if kind == NodeKind::AssignStmt
        && let Some(place) = cst::nodes(tree, node).first().copied()
        && tree.kind(place) == NodeKind::NameExpr
        && let Some((name, _)) = cst::name(tree, source, place)
    {
        facts.assigned.insert(name);
    }

    if in_lambda
        && kind == NodeKind::NameExpr
        && let Some((name, _)) = cst::name(tree, source, node)
    {
        facts.captured.insert(name);
    }

    let inside = in_lambda || kind == NodeKind::LambdaExpr;
    for child in cst::nodes(tree, node) {
        collect(tree, source, child, inside, facts);
    }
}

/// The names a lambda mentions and does not itself bind, in first-mention
/// order.
///
/// The order is the closure environment's, so it has to be deterministic;
/// first mention is as good as any and reads well in a dump.
///
/// A nested `fn` is not descended into. §5 says a `fn` captures nothing, so a
/// name it mentions is not the enclosing lambda's to capture — and if that name
/// turns out to be an enclosing local, the `fn`'s own lowering is where the
/// complaint belongs.
pub fn free_vars(tree: &Tree, source: &str, lambda: NodeId) -> Vec<String> {
    let mut walker = FreeVars {
        tree,
        source,
        bound: Vec::new(),
        free: Vec::new(),
        seen: HashSet::new(),
    };
    walker.lambda(lambda);
    walker.free
}

struct FreeVars<'a> {
    tree: &'a Tree,
    source: &'a str,
    bound: Vec<HashSet<String>>,
    free: Vec<String>,
    seen: HashSet<String>,
}

impl FreeVars<'_> {
    fn lambda(&mut self, node: NodeId) {
        let children = cst::nodes(self.tree, node);
        self.bound.push(HashSet::new());
        for child in &children {
            if self.tree.kind(*child) == NodeKind::LambdaParamList {
                for param in cst::nodes(self.tree, *child) {
                    if let Some((name, _)) = cst::name(self.tree, self.source, param) {
                        self.bind(name);
                    }
                }
            }
        }
        for child in &children {
            if cst::is_expr(self.tree.kind(*child)) {
                self.walk(*child);
            }
        }
        self.bound.pop();
    }

    fn walk(&mut self, node: NodeId) {
        match self.tree.kind(node) {
            NodeKind::BlockExpr | NodeKind::SourceFile => {
                self.bound.push(HashSet::new());
                let children = cst::nodes(self.tree, node);
                // Declarations are hoisted (§3, §5), so they are in scope for
                // the whole block, statements above them included.
                for child in &children {
                    if cst::is_declaration(self.tree.kind(*child))
                        && let Some((name, _)) = cst::name(self.tree, self.source, *child)
                    {
                        self.bind(name);
                    }
                }
                for child in &children {
                    self.walk(*child);
                }
                self.bound.pop();
            }
            NodeKind::LetStmt => {
                // §3: the initializer is evaluated before the binding exists,
                // so `let x = x;` names the outer `x`.
                for child in cst::expr_children(self.tree, node) {
                    self.walk(child);
                }
                for child in cst::nodes(self.tree, node) {
                    if self.tree.kind(child) == NodeKind::NamePat
                        && let Some((name, _)) = cst::name(self.tree, self.source, child)
                    {
                        self.bind(name);
                    }
                }
            }
            NodeKind::LambdaExpr => self.lambda(node),
            // Bound above by the hoisting pass, and not descended into.
            NodeKind::FnDecl | NodeKind::StructDecl | NodeKind::TypeAliasDecl => {}
            NodeKind::NameExpr => {
                if let Some((name, _)) = cst::name(self.tree, self.source, node) {
                    self.use_name(name);
                }
            }
            // The leading name is a *type*, so it is not a value reference.
            NodeKind::StructLitExpr => {
                for child in cst::nodes(self.tree, node) {
                    if self.tree.kind(child) != NodeKind::NameExpr {
                        self.walk(child);
                    }
                }
            }
            _ => {
                for child in cst::nodes(self.tree, node) {
                    self.walk(child);
                }
            }
        }
    }

    fn bind(&mut self, name: String) {
        if let Some(scope) = self.bound.last_mut() {
            scope.insert(name);
        }
    }

    fn use_name(&mut self, name: String) {
        if self.bound.iter().any(|scope| scope.contains(&name)) {
            return;
        }
        if self.seen.insert(name.clone()) {
            self.free.push(name);
        }
    }
}
