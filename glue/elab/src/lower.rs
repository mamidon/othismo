//! Elaboration: concrete syntax tree in, core IR out.
//!
//! Name resolution, type checking, A-normal form, capture analysis, and slot
//! allocation happen in one pass. §comptime says they must — an annotation can
//! need a call evaluated before its type exists, so the passes are mutually
//! recursive and admit no ordering. What makes that tractable is §inference's
//! choice of local inference over whole-program Hindley–Milner: every `fn`
//! signature is annotated (§functions), so a body can be elaborated knowing
//! only its own signature and the declarations it names.
//!
//! The CST is read and never rewritten. It is the language server's tree and
//! has to keep the property `parser` opens with — every byte reachable — and a
//! derived node has no source bytes to be reachable from. Core IR carries
//! provenance back instead.
//!
//! # Nested blocks are flattened
//!
//! A `{ … }` used as an expression lowers into the *enclosing* [`Block`] rather
//! than becoming one of its own. Scopes are gone by this point, so a block
//! boundary carries no meaning that survives lowering, and a `Stmt::Block`
//! would be a node with nothing to say. Only the arms of `if` and the two
//! halves of `while` get blocks, because control flow needs somewhere to branch
//! to.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use ir::consts::{Const, ConstId};
use ir::program::{
    BinOp, Block, BlockId, CstId, Func, FuncId, GlobalDef, GlobalId, Operand, Place, Program,
    Rvalue, Slot, SlotDef, SlotKind, Stmt, UnOp,
};
use ir::sym::{Interner, Sym};
use ir::types::{FieldDef, FieldIdx, TypeDef, TypeId, Types};
use parser::{NodeId, NodeKind, Tree};
use tokenizer::{Literal, NumericType, Span, TokenKind};

use crate::cst;
use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::scan::{self, BlockFacts};

pub struct Lowered {
    pub program: Program,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lower(tree: &Tree, source: &str) -> Lowered {
    Lowerer::new(tree, source).run()
}

/// The result of checking an expression.
///
/// The two constant variants are §lexical's unpinned constants: a mathematical
/// value with no type yet, which acquires one at the point it becomes a
/// runtime value. They are not an optimization — `let n = 3; n - 5` is `-2`
/// because of them, where pinning at the `let` would make it an underflow.
#[derive(Clone, Debug)]
enum Checked {
    Val(Operand, TypeId),
    /// An unpinned integer constant. `i128` stands in for §lexical's unbounded
    /// precision; exceeding it is [`DiagnosticKind::ConstantOverflow`], which
    /// is a smaller lie than wrapping would be.
    Int(i128),
    Float(f64),
    /// A diagnostic has already been reported here. Poison, so that one mistake
    /// produces one message.
    Error,
}

#[derive(Clone, Debug)]
enum Binding {
    Value {
        slot: Slot,
        /// The type of the *value*. When `cell` is set, the slot itself holds a
        /// `Cell(ty)`.
        ty: TypeId,
        cell: bool,
        mutable: bool,
    },
    /// A binding whose initializer was an unpinned constant and which is never
    /// assigned (§lexical). It has no slot and no type until it is used.
    Const(Checked),
    /// §statements' top-level binding. Storage that outlives every frame, so
    /// reading one from inside a `fn` is not a capture and needs no
    /// permission — see [`GlobalId`].
    Global {
        global: GlobalId,
        ty: TypeId,
        mutable: bool,
    },
    Func {
        id: FuncId,
        ty: TypeId,
    },
    Type(TypeId),
}

struct Scope {
    /// Index into `Lowerer::funcs`. A value binding found in a scope belonging
    /// to an outer function is a capture, or an error if the inner one is a
    /// `fn`.
    func: usize,
    names: HashMap<String, Binding>,
    facts: Option<BlockFacts>,
}

struct FuncCtx {
    id: FuncId,
    name: Option<Sym>,
    slots: Vec<SlotDef>,
    blocks: Vec<Block>,
    ret: TypeId,
    /// False while a lambda's return type is still being inferred (§functions:
    /// a lambda's types come from context, and sometimes there isn't any).
    ret_known: bool,
    loop_depth: u32,
    /// Names the dump gives to this function's lambdas: `outer.λ0`, `outer.λ1`.
    lambdas: u32,
    origin: CstId,
}

struct Lowerer<'a> {
    tree: &'a Tree,
    source: &'a str,
    program: Program,
    diagnostics: Vec<Diagnostic>,
    scopes: Vec<Scope>,
    funcs: Vec<FuncCtx>,
    /// Signatures, resolved once at hoist time and read again when the body is
    /// lowered — otherwise every problem in a signature is reported twice.
    signatures: HashMap<NodeId, (Vec<ParamInfo>, TypeId)>,
    /// Each `fn`'s parameter names and their `mut` flags, so a call site can
    /// check §functions' rule. Kept here rather than read back off the
    /// callee's [`SlotDef`]s, because a call may be lowered before the
    /// callee's body is — that is what hoisting is for — and a slot does not
    /// exist until then.
    ///
    /// Nothing records the same for a function *value*: §functions gives a
    /// `fn` type no `mut`, so an indirect call has nothing to check against.
    /// See [`Lowerer::call`].
    mut_params: HashMap<FuncId, Vec<(String, bool)>>,

    t_unit: TypeId,
    t_bool: TypeId,
    t_char: TypeId,
    t_str: TypeId,
    t_u64: TypeId,
    t_s64: TypeId,
    t_f64: TypeId,
    t_error: TypeId,
    c_unit: ConstId,
}

impl<'a> Lowerer<'a> {
    fn new(tree: &'a Tree, source: &'a str) -> Lowerer<'a> {
        let mut types = Types::new();
        let t_unit = types.intern(TypeDef::Unit);
        let t_bool = types.intern(TypeDef::Bool);
        let t_char = types.intern(TypeDef::Char);
        let t_str = types.intern(TypeDef::Str);
        let t_u64 = types.intern(TypeDef::Int {
            signed: false,
            bits: 64,
        });
        let t_s64 = types.intern(TypeDef::Int {
            signed: true,
            bits: 64,
        });
        let t_f64 = types.intern(TypeDef::Float { bits: 64 });
        let t_error = types.intern(TypeDef::Error);

        let mut consts = ir::consts::ConstPool::new();
        let c_unit = consts.add(Const::Unit);

        Lowerer {
            tree,
            source,
            program: Program {
                funcs: Vec::new(),
                globals: Vec::new(),
                types,
                consts,
                syms: Interner::new(),
                entry: None,
            },
            diagnostics: Vec::new(),
            scopes: Vec::new(),
            funcs: Vec::new(),
            signatures: HashMap::new(),
            mut_params: HashMap::new(),
            t_unit,
            t_bool,
            t_char,
            t_str,
            t_u64,
            t_s64,
            t_f64,
            t_error,
            c_unit,
        }
    }

    fn run(mut self) -> Lowered {
        let root = self.tree.root();
        let name = self.program.syms.intern("<file>");

        // §statements: a file is a block. Its trailing expression is the
        // file's value, which makes goal §one-language's "a bare expression is
        // a valid program" the block rule applied to the outermost block
        // rather than a REPL rule.
        let id = self.begin_func(Some(name), self.t_error, root);
        self.push_scope(None);
        self.declare_prelude();

        let body = self.new_block();
        let value = self.block_into(body, root, None);
        let (operand, ty) = self.pin(value, None, root);
        self.cur_mut().ret = ty;
        self.cur_mut().ret_known = true;
        self.emit_result(body, operand, root);

        self.pop_scope();
        self.end_func();
        self.program.entry = Some(id);

        for (global, via, at) in init_problems(&self.program, id) {
            let kind = DiagnosticKind::UninitializedGlobal {
                global: self
                    .program
                    .text(self.program.global(global).name)
                    .to_string(),
                via: via
                    .and_then(|func| self.program.func(func).name)
                    .map(|name| self.program.text(name).to_string()),
            };
            self.error(kind, at);
        }

        Lowered {
            program: self.program,
            diagnostics: self.diagnostics,
        }
    }

    /// §lexical: type names are ordinary identifiers, not keywords, so they
    /// live in the outermost scope and a program may shadow them.
    fn declare_prelude(&mut self) {
        let scalars: Vec<(String, TypeId)> = vec![
            ("bool".into(), self.t_bool),
            ("char".into(), self.t_char),
            ("Str".into(), self.t_str),
            ("f64".into(), self.t_f64),
        ];
        for (name, ty) in scalars {
            self.bind(name, Binding::Type(ty));
        }
        let f32 = self.program.types.intern(TypeDef::Float { bits: 32 });
        self.bind("f32".into(), Binding::Type(f32));
        for bits in [8u8, 16, 32, 64] {
            for signed in [false, true] {
                let ty = self.program.types.intern(TypeDef::Int { signed, bits });
                let name = format!("{}{}", if signed { "s" } else { "u" }, bits);
                self.bind(name, Binding::Type(ty));
            }
        }
    }

    // ---- Diagnostics -------------------------------------------------------

    fn error(&mut self, kind: DiagnosticKind, at: NodeId) {
        // The node's *significant* extent: the tree is lossless, so a node
        // begins at whatever trivia was attached to its first token, and a
        // caret under a blank line names nothing.
        let span = self.tree.significant_span(at);
        self.diagnostics.push(Diagnostic::new(kind, span));
    }

    fn error_at(&mut self, kind: DiagnosticKind, span: Span) {
        self.diagnostics.push(Diagnostic::new(kind, span));
    }

    fn type_name(&self, ty: TypeId) -> String {
        ir::program::type_name(&self.program.types, &self.program.syms, ty)
    }

    // ---- Functions and blocks ---------------------------------------------

    fn begin_func(&mut self, name: Option<Sym>, ret: TypeId, origin: NodeId) -> FuncId {
        let id = FuncId(self.program.funcs.len() as u32);
        // A placeholder, so the id exists while the body is being built and a
        // function can call itself.
        self.program.funcs.push(Func {
            name,
            params: 0,
            captures: 0,
            ret,
            slots: Vec::new(),
            blocks: Vec::new(),
            origin,
        });
        self.funcs.push(FuncCtx {
            id,
            name,
            slots: Vec::new(),
            blocks: Vec::new(),
            ret,
            ret_known: true,
            loop_depth: 0,
            lambdas: 0,
            origin,
        });
        id
    }

    fn end_func(&mut self) {
        let mut ctx = self.funcs.pop().expect("end_func without begin_func");
        compact_slots(&mut ctx);
        let params = ctx
            .slots
            .iter()
            .filter(|slot| slot.kind == SlotKind::Param)
            .count() as u32;
        let captures = ctx
            .slots
            .iter()
            .filter(|slot| slot.kind == SlotKind::Capture)
            .count() as u32;
        self.program.funcs[ctx.id.index()] = Func {
            name: ctx.name,
            params,
            captures,
            ret: ctx.ret,
            slots: ctx.slots,
            blocks: ctx.blocks,
            origin: ctx.origin,
        };
    }

    fn cur(&self) -> &FuncCtx {
        self.funcs.last().expect("no function being lowered")
    }

    fn cur_mut(&mut self) -> &mut FuncCtx {
        self.funcs.last_mut().expect("no function being lowered")
    }

    fn new_block(&mut self) -> BlockId {
        let ctx = self.cur_mut();
        let id = BlockId(ctx.blocks.len() as u32);
        ctx.blocks.push(Block::new());
        id
    }

    fn emit(&mut self, blk: BlockId, stmt: Stmt, at: NodeId) {
        self.cur_mut().blocks[blk.index()].push(stmt, at);
    }

    /// Emits the function's result, unless the body already ended in `return`.
    ///
    /// §functions: a body is a block, so its ordinary result is its trailing
    /// expression and `return` is for early exit. A function using the early
    /// exit as its *only* exit would otherwise get a second, unreachable
    /// return after it.
    fn emit_result(&mut self, blk: BlockId, operand: Operand, at: NodeId) {
        if matches!(
            self.cur().blocks[blk.index()].stmts.last(),
            Some(Stmt::Return(_))
        ) {
            return;
        }
        self.emit(blk, Stmt::Return(Some(operand)), at);
    }

    fn slot(&mut self, ty: TypeId, name: Option<Sym>, kind: SlotKind, mutable: bool) -> Slot {
        let ctx = self.cur_mut();
        let slot = Slot(ctx.slots.len() as u32);
        ctx.slots.push(SlotDef {
            ty,
            name,
            kind,
            mutable,
        });
        slot
    }

    /// Whether a binding declared here is one of §statements' top-level
    /// bindings: the outermost block of the file, and so of the entry
    /// function. Two scopes are open there — the prelude's and the file's.
    fn at_file_scope(&self) -> bool {
        self.funcs.len() == 1 && self.scopes.len() == 2
    }

    fn new_global(&mut self, name: Sym, ty: TypeId, mutable: bool, origin: NodeId) -> GlobalId {
        let id = GlobalId(self.program.globals.len() as u32);
        self.program.globals.push(GlobalDef {
            name,
            ty,
            mutable,
            origin,
        });
        id
    }

    fn temp(&mut self, ty: TypeId) -> Slot {
        // §statements' `mut` is about a binding, and a temporary is not one.
        self.slot(ty, None, SlotKind::Temp, false)
    }

    /// Assigns an operand into a slot, folding away the copy when the operand
    /// is a temporary this block just produced.
    ///
    /// `i = i + 1` would otherwise be `t = i + 1` followed by `i = t`, because
    /// ANF names every intermediate and only then discovers where it is going.
    /// Retargeting the instruction that made the temporary is exact rather than
    /// an optimization: the temporary has one definition, one use, and no name
    /// anyone could observe it through.
    fn assign_into(&mut self, blk: BlockId, dst: Slot, operand: Operand, at: NodeId) {
        if let Operand::Slot(temp) = operand
            && self.cur().slots[temp.index()].kind == SlotKind::Temp
            && let Some(Stmt::Assign { dst: last, .. }) =
                self.cur().blocks[blk.index()].stmts.last()
            && *last == temp
        {
            let ctx = self.cur_mut();
            if let Some(Stmt::Assign { dst: last, .. }) = ctx.blocks[blk.index()].stmts.last_mut() {
                *last = dst;
                // The temporary was allocated immediately before the
                // instruction that filled it, so it is still the last slot and
                // nothing else can refer to it.
                if temp.index() + 1 == ctx.slots.len() {
                    ctx.slots.pop();
                }
                return;
            }
        }
        self.emit(
            blk,
            Stmt::Assign {
                dst,
                rvalue: Rvalue::Use(operand),
            },
            at,
        );
    }

    /// Assigns an rvalue into a fresh temporary — the shape almost every
    /// expression takes in A-normal form.
    fn emit_temp(&mut self, blk: BlockId, rvalue: Rvalue, ty: TypeId, at: NodeId) -> Checked {
        let dst = self.temp(ty);
        self.emit(blk, Stmt::Assign { dst, rvalue }, at);
        Checked::Val(Operand::Slot(dst), ty)
    }

    // ---- Scopes ------------------------------------------------------------

    fn push_scope(&mut self, facts: Option<BlockFacts>) {
        let func = self.funcs.len() - 1;
        self.scopes.push(Scope {
            func,
            names: HashMap::new(),
            facts,
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: String, binding: Binding) {
        self.scopes
            .last_mut()
            .expect("no scope")
            .names
            .insert(name, binding);
    }

    /// The innermost binding of `name`, and which function's scope it was found
    /// in.
    fn resolve(&self, name: &str) -> Option<(usize, Binding)> {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.names.get(name) {
                return Some((scope.func, binding.clone()));
            }
        }
        None
    }

    /// The facts of the innermost block scope, for §lexical's pinning rule and
    /// §functions' cell rule.
    fn facts_needs_cell(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.facts.as_ref())
            .is_some_and(|facts| facts.needs_cell(name))
    }

    fn facts_assigned(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.facts.as_ref())
            .is_some_and(|facts| facts.assigned.contains(name))
    }

    // ---- Constants ---------------------------------------------------------

    fn int_const(&mut self, value: i128, ty: TypeId) -> Operand {
        let bits = value as u64;
        Operand::Const(self.program.consts.add(Const::Int { ty, bits }))
    }

    fn float_const(&mut self, value: f64, ty: TypeId) -> Operand {
        let value = if matches!(self.program.types.get(ty), TypeDef::Float { bits: 32 }) {
            value as f32 as f64
        } else {
            value
        };
        Operand::Const(self.program.consts.add(Const::Float {
            ty,
            bits: value.to_bits(),
        }))
    }

    fn fits(&self, value: i128, ty: TypeId) -> bool {
        match *self.program.types.get(ty) {
            TypeDef::Int {
                signed: false,
                bits,
            } => value >= 0 && (bits == 64 || value < (1i128 << bits)),
            TypeDef::Int { signed: true, bits } => {
                let limit = 1i128 << (bits - 1);
                value >= -limit && value < limit
            }
            _ => false,
        }
    }

    /// §lexical's pinning rule, in order: context wins; otherwise sign
    /// decides; otherwise it is an error.
    fn pin(&mut self, value: Checked, expect: Option<TypeId>, at: NodeId) -> (Operand, TypeId) {
        match value {
            Checked::Val(operand, ty) => {
                if let Some(want) = expect
                    && !self.program.types.compatible(ty, want)
                {
                    let expected = self.type_name(want);
                    let found = self.type_name(ty);
                    self.error(DiagnosticKind::TypeMismatch { expected, found }, at);
                    return (operand, self.t_error);
                }
                (operand, ty)
            }
            Checked::Int(v) => {
                let ty = match expect {
                    Some(want) if self.program.types.is_integer(want) => want,
                    Some(want) if self.program.types.is_float(want) => {
                        return (self.float_const(v as f64, want), want);
                    }
                    Some(want) if self.program.types.is_error(want) => return self.pinned_error(),
                    Some(want) => {
                        let expected = self.type_name(want);
                        self.error(
                            DiagnosticKind::TypeMismatch {
                                expected,
                                found: "an integer constant".to_string(),
                            },
                            at,
                        );
                        return self.pinned_error();
                    }
                    None if v >= 0 => self.t_u64,
                    None => self.t_s64,
                };
                if !self.fits(v, ty) {
                    let name = self.type_name(ty);
                    self.error(
                        DiagnosticKind::ConstantOutOfRange {
                            value: v.to_string(),
                            ty: name,
                        },
                        at,
                    );
                    return self.pinned_error();
                }
                (self.int_const(v, ty), ty)
            }
            Checked::Float(v) => {
                let ty = match expect {
                    Some(want) if self.program.types.is_float(want) => want,
                    Some(want) if self.program.types.is_error(want) => return self.pinned_error(),
                    Some(want) => {
                        let expected = self.type_name(want);
                        self.error(
                            DiagnosticKind::TypeMismatch {
                                expected,
                                found: "a float constant".to_string(),
                            },
                            at,
                        );
                        return self.pinned_error();
                    }
                    None => self.t_f64,
                };
                (self.float_const(v, ty), ty)
            }
            Checked::Error => self.pinned_error(),
        }
    }

    fn pinned_error(&self) -> (Operand, TypeId) {
        (Operand::Const(self.c_unit), self.t_error)
    }

    /// The type a checked value already has, where one is known without
    /// pinning. Used to give the right operand of a binary operator a hint.
    fn known_type(&self, value: &Checked) -> Option<TypeId> {
        match value {
            Checked::Val(_, ty) => Some(*ty),
            _ => None,
        }
    }
}

// ---- Blocks and statements -------------------------------------------------

impl Lowerer<'_> {
    /// Lowers a block's statements into `blk` and returns the block's value.
    ///
    /// §expressions' semicolon rule: the value is the trailing expression,
    /// written without a `;`. A block with none is unit.
    fn block_into(&mut self, blk: BlockId, node: NodeId, expect: Option<TypeId>) -> Checked {
        let facts = scan::block_facts(self.tree, self.source, node);
        self.push_scope(Some(facts));

        let children = cst::nodes(self.tree, node);
        self.hoist(&children);

        let mut value = Checked::Val(Operand::Const(self.c_unit), self.t_unit);
        for child in &children {
            let kind = self.tree.kind(*child);
            if kind == NodeKind::FnDecl {
                continue;
            }
            if cst::is_expr(kind) {
                // The trailing expression, and so the block's value
                // (§expressions, §statements).
                value = self.expr(blk, *child, expect);
            } else {
                self.stmt(blk, *child);
            }
        }

        // §statements: a `fn` body is walked after the rest of its block, so
        // it sees every binding the block declares rather than only the ones
        // written above it. That is the whole of "declarations hoist while
        // initializers still run in order" — a body emits nothing into the
        // enclosing block, so moving *when* it is walked changes name
        // resolution and no code order whatsoever.
        for child in &children {
            if self.tree.kind(*child) == NodeKind::FnDecl {
                self.fn_body(*child);
            }
        }

        self.pop_scope();
        value
    }

    /// §statements and §functions: declarations are order-independent within
    /// their block, so that mutual recursion works while statements still run
    /// in order. §scope owes the general rule; hoisting per block is the
    /// smallest thing that covers the cases that exist, and it is what the
    /// interpreter does too.
    ///
    /// Four sub-passes, and the order between them matters: a struct's fields
    /// may name an alias, an alias may name a struct, and a signature may name
    /// either. Allocating every struct's [`TypeId`] first makes all three legal
    /// without a dependency sort.
    fn hoist(&mut self, children: &[NodeId]) {
        let mut structs = Vec::new();
        for child in children {
            if self.tree.kind(*child) == NodeKind::StructDecl
                && let Some((name, _)) = cst::name(self.tree, self.source, *child)
            {
                let sym = self.program.syms.intern(&name);
                let ty = self.program.types.fresh_struct(Some(sym), *child);
                self.bind(name, Binding::Type(ty));
                structs.push((*child, ty));
            }
        }

        for child in children {
            if self.tree.kind(*child) == NodeKind::TypeAliasDecl
                && let Some((name, _)) = cst::name(self.tree, self.source, *child)
            {
                // §types: an alias is a second name for one type, not a new
                // one.
                let ty = match cst::type_child(self.tree, *child) {
                    Some(node) => self.resolve_type(node),
                    None => self.t_error,
                };
                self.bind(name, Binding::Type(ty));
            }
        }

        for (node, ty) in structs {
            let fields = self.struct_fields(node);
            self.program.types.set_fields(ty, fields);
        }

        for child in children {
            if self.tree.kind(*child) == NodeKind::FnDecl {
                self.hoist_fn(*child);
            }
        }
    }

    fn struct_fields(&mut self, node: NodeId) -> Vec<FieldDef> {
        let mut fields: Vec<FieldDef> = Vec::new();
        for list in cst::nodes(self.tree, node) {
            if self.tree.kind(list) != NodeKind::FieldDeclList {
                continue;
            }
            for decl in cst::nodes(self.tree, list) {
                let Some((name, span)) = cst::name(self.tree, self.source, decl) else {
                    continue;
                };
                if fields
                    .iter()
                    .any(|field| self.program.syms.text(field.name) == name)
                {
                    self.error_at(DiagnosticKind::DuplicateField(name), span);
                    continue;
                }
                // §types: field types are required — there is no inference
                // across a declaration boundary.
                let ty = match cst::type_child(self.tree, decl) {
                    Some(node) => self.resolve_type(node),
                    None => self.t_error,
                };
                let sym = self.program.syms.intern(&name);
                fields.push(FieldDef { name: sym, ty });
            }
        }
        fields
    }

    /// Resolves a `fn`'s signature and reserves its [`FuncId`]. The body is
    /// lowered later, in source order, so a diagnostic inside it follows the
    /// ones above it.
    fn hoist_fn(&mut self, node: NodeId) {
        let Some((name, _)) = cst::name(self.tree, self.source, node) else {
            return;
        };
        let (params, ret) = self.fn_signature(node);
        self.signatures.insert(node, (params.clone(), ret));
        let ty = self.program.types.intern(TypeDef::Fn {
            params: params.iter().map(|param| param.ty).collect(),
            ret,
        });
        let sym = self.program.syms.intern(&name);
        let id = FuncId(self.program.funcs.len() as u32);
        self.program.funcs.push(Func {
            name: Some(sym),
            params: params.len() as u32,
            captures: 0,
            ret,
            slots: Vec::new(),
            blocks: Vec::new(),
            origin: node,
        });
        self.mut_params.insert(
            id,
            params
                .iter()
                .map(|param| (param.name.clone(), param.mutable))
                .collect(),
        );
        self.bind(name, Binding::Func { id, ty });
    }

    fn fn_signature(&mut self, node: NodeId) -> (Vec<ParamInfo>, TypeId) {
        let mut params = Vec::new();
        let mut ret = self.t_unit;
        for child in cst::nodes(self.tree, node) {
            match self.tree.kind(child) {
                NodeKind::ParamList => {
                    for param in cst::nodes(self.tree, child) {
                        let name = cst::name(self.tree, self.source, param)
                            .map(|(name, _)| name)
                            .unwrap_or_default();
                        let ty = match cst::type_child(self.tree, param) {
                            Some(node) => self.resolve_type(node),
                            None => self.t_error,
                        };
                        // §functions: `mut` belongs to the parameter, not the
                        // type.
                        let mutable = cst::has_token(self.tree, param, TokenKind::Mut);
                        // §comptime is decided and unbuilt: a comptime
                        // parameter would make this function a generic, and
                        // instantiating one needs `Type` in the value domain
                        // and a cache keyed on comptime arguments. Reported
                        // here rather than at the call, because it is the
                        // declaration that cannot be given a meaning.
                        if cst::has_token(self.tree, param, TokenKind::Comptime) {
                            self.error(DiagnosticKind::Unsupported("comptime"), param);
                        }
                        params.push(ParamInfo { name, ty, mutable });
                    }
                }
                // §functions: omitted when the return type is unit.
                NodeKind::RetType => {
                    ret = match cst::type_child(self.tree, child) {
                        Some(node) => self.resolve_type(node),
                        None => self.t_error,
                    };
                }
                _ => {}
            }
        }
        (params, ret)
    }

    fn stmt(&mut self, blk: BlockId, node: NodeId) {
        match self.tree.kind(node) {
            NodeKind::LetStmt => self.let_stmt(blk, node),
            NodeKind::AssignStmt => self.assign_stmt(blk, node),
            NodeKind::ExprStmt => self.expr_stmt(blk, node),
            NodeKind::WhileStmt => self.while_stmt(blk, node),
            NodeKind::BreakStmt | NodeKind::ContinueStmt => self.jump_stmt(blk, node),
            NodeKind::ReturnStmt => self.return_stmt(blk, node),
            // Signature hoisted, body walked at the end of the block by
            // `block_into`. Nothing runs at its position either way.
            NodeKind::FnDecl => {}
            // Hoisted, and nothing runs at their position.
            NodeKind::StructDecl | NodeKind::TypeAliasDecl => {}
            // The parser already reported it and kept every byte in its tree.
            // There is nothing here to elaborate and nothing to add.
            NodeKind::Error => {}
            _ => {}
        }
    }

    fn let_stmt(&mut self, blk: BlockId, node: NodeId) {
        let annotation = cst::type_child(self.tree, node).map(|ty| self.resolve_type(ty));
        let init = cst::expr_children(self.tree, node).last().copied();
        let name = cst::nodes(self.tree, node)
            .into_iter()
            .find(|child| self.tree.kind(*child) == NodeKind::NamePat)
            .and_then(|pat| cst::name(self.tree, self.source, pat))
            .map(|(name, _)| name);

        let Some(name) = name else { return };
        let value = match init {
            Some(init) => self.expr(blk, init, annotation),
            None => Checked::Error,
        };

        // §lexical: a binding stays an unpinned constant when its initializer
        // is a constant expression and it is never assigned in its scope. Both
        // conditions are syntactic, which is the point — a rule needing
        // dataflow is a rule two back ends eventually disagree about.
        if annotation.is_none()
            && matches!(value, Checked::Int(_) | Checked::Float(_))
            && !self.facts_assigned(&name)
        {
            self.bind(name, Binding::Const(value));
            return;
        }

        let (operand, ty) = self.pin(value, annotation, init.unwrap_or(node));
        let sym = self.program.syms.intern(&name);
        let mutable = cst::has_token(self.tree, node, TokenKind::Mut);
        let cell = self.facts_needs_cell(&name);

        // §statements: a top-level binding is a global, not a local of the
        // entry function. That is what lets a `fn` read it — a global is
        // storage rather than a frame, so reaching one is not a capture and
        // §functions' "a `fn` carries no environment" survives untouched. It
        // also needs no cell: a cell exists to give a captured binding a home
        // that outlives its frame, and a global already has one.
        if self.at_file_scope() {
            let global = self.new_global(sym, ty, mutable, node);
            self.emit(
                blk,
                Stmt::Store {
                    place: Place::Global(global),
                    value: operand,
                },
                node,
            );
            self.bind(
                name,
                Binding::Global {
                    global,
                    ty,
                    mutable,
                },
            );
            return;
        }

        let slot = if cell {
            let cell_ty = self.program.types.intern(TypeDef::Cell(ty));
            let slot = self.slot(cell_ty, Some(sym), SlotKind::Local, mutable);
            self.emit(
                blk,
                Stmt::Assign {
                    dst: slot,
                    rvalue: Rvalue::MakeCell(operand),
                },
                node,
            );
            slot
        } else {
            let slot = self.slot(ty, Some(sym), SlotKind::Local, mutable);
            self.assign_into(blk, slot, operand, node);
            slot
        };

        // §statements: the initializer is evaluated before the binding exists,
        // so binding *after* lowering it is what makes `let x = x;` name the
        // outer one.
        self.bind(
            name,
            Binding::Value {
                slot,
                ty,
                cell,
                mutable,
            },
        );
    }

    fn assign_stmt(&mut self, blk: BlockId, node: NodeId) {
        let children = cst::expr_children(self.tree, node);
        let (Some(&place), Some(&value)) = (children.first(), children.get(1)) else {
            return;
        };

        match self.tree.kind(place) {
            NodeKind::NameExpr => {
                let Some((name, span)) = cst::name(self.tree, self.source, place) else {
                    return;
                };
                let Some((_, binding)) = self.resolve(&name) else {
                    self.error_at(DiagnosticKind::UnknownName(name), span);
                    return;
                };
                // §statements: assigning a global is the same statement as
                // initializing one, and rebinding needs no permission.
                if let Binding::Global { global, ty, .. } = binding {
                    let checked = self.expr(blk, value, Some(ty));
                    let (operand, _) = self.pin(checked, Some(ty), value);
                    self.emit(
                        blk,
                        Stmt::Store {
                            place: Place::Global(global),
                            value: operand,
                        },
                        node,
                    );
                    return;
                }
                let Binding::Value { slot, ty, cell, .. } = binding else {
                    self.error(DiagnosticKind::NotAPlace, place);
                    return;
                };
                // §statements: assignment must match the binding's type. To
                // give a name a value of a different type, declare it again —
                // that is shadowing, and it is why both forms stay useful.
                let checked = self.expr(blk, value, Some(ty));
                let (operand, _) = self.pin(checked, Some(ty), value);
                if cell {
                    self.emit(
                        blk,
                        Stmt::Store {
                            place: Place::Cell(slot),
                            value: operand,
                        },
                        node,
                    );
                } else {
                    self.assign_into(blk, slot, operand, node);
                }
            }
            NodeKind::FieldExpr => {
                // §types: field mutability follows the binding. A `mut`
                // binding permits assigning any field; a non-`mut` one permits
                // none.
                if let Some((root, span)) = self.place_root(place)
                    && let Some((_, binding)) = self.resolve(&root)
                    && matches!(
                        binding,
                        Binding::Value { mutable: false, .. }
                            | Binding::Global { mutable: false, .. }
                    )
                {
                    self.error_at(DiagnosticKind::AssignToNonMut(root), span);
                }
                let Some((base, field, field_ty)) = self.field_access(blk, place) else {
                    return;
                };
                let checked = self.expr(blk, value, Some(field_ty));
                let (operand, _) = self.pin(checked, Some(field_ty), value);
                let Operand::Slot(base) = base else {
                    self.error(DiagnosticKind::NotAPlace, place);
                    return;
                };
                self.emit(
                    blk,
                    Stmt::Store {
                        place: Place::Field { base, field },
                        value: operand,
                    },
                    node,
                );
            }
            NodeKind::IndexExpr => {
                self.error(DiagnosticKind::Unsupported("indexing"), place);
            }
            _ => self.error(DiagnosticKind::NotAPlace, place),
        }
    }

    /// The name at the root of a place expression — the binding whose `mut`
    /// governs the whole chain.
    fn place_root(&self, node: NodeId) -> Option<(String, Span)> {
        let mut current = node;
        loop {
            match self.tree.kind(current) {
                NodeKind::NameExpr => return cst::name(self.tree, self.source, current),
                NodeKind::FieldExpr | NodeKind::IndexExpr | NodeKind::ParenExpr => {
                    current = *cst::nodes(self.tree, current).first()?;
                }
                _ => return None,
            }
        }
    }

    fn expr_stmt(&mut self, blk: BlockId, node: NodeId) {
        let Some(&inner) = cst::expr_children(self.tree, node).first() else {
            return;
        };
        // A call is the one expression worth discarding explicitly: everything
        // else has already landed in a temporary nobody reads.
        if self.tree.kind(inner) == NodeKind::CallExpr {
            if let Some((rvalue, _)) = self.call(blk, inner) {
                self.emit(blk, Stmt::Drop(rvalue), node);
            }
            return;
        }
        self.expr(blk, inner, None);
    }

    fn while_stmt(&mut self, blk: BlockId, node: NodeId) {
        // By position: the condition, then the body. Finding the body by kind
        // would find the condition instead when the condition is itself a
        // block — which is the only way to write a statement in one, and so
        // the only way to write a jump there.
        let children = cst::expr_children(self.tree, node);
        let cond_node = children.first().copied();
        let body_node = children
            .get(1)
            .copied()
            .filter(|child| self.tree.kind(*child) == NodeKind::BlockExpr);

        // The header re-runs every iteration, so the condition's computation
        // belongs inside it rather than before the loop. That is ANF's price,
        // and it is visible right here.
        //
        // The condition is lowered *inside* the loop for §control's purposes:
        // a jump written in it — which needs a block expression, so `while {
        // break; true } { … }` — has this loop as its innermost enclosing one,
        // since this is the loop it conditions. The header is where it lands,
        // and both back ends leave the loop from there.
        let header = self.new_block();
        self.cur_mut().loop_depth += 1;
        let cond = match cond_node {
            Some(cond_node) => {
                let checked = self.expr(header, cond_node, Some(self.t_bool));
                self.condition(checked, cond_node)
            }
            None => Operand::Const(self.c_unit),
        };
        self.cur_mut().loop_depth -= 1;

        let body = self.new_block();
        if let Some(body_node) = body_node {
            self.cur_mut().loop_depth += 1;
            // §control: a loop is a statement and its value is unit.
            self.block_into(body, body_node, None);
            self.cur_mut().loop_depth -= 1;
        }

        self.emit(blk, Stmt::While { header, cond, body }, node);
    }

    /// §expressions and §control: a condition must be `bool`. Not "convertible
    /// to" — there are no implicit conversions and no truthiness, so a `bool`
    /// is the only thing that can appear here.
    fn condition(&mut self, checked: Checked, at: NodeId) -> Operand {
        match checked {
            Checked::Val(operand, ty) if self.program.types.compatible(ty, self.t_bool) => operand,
            Checked::Val(_, ty) => {
                let found = self.type_name(ty);
                self.error(DiagnosticKind::ConditionNotBool(found), at);
                Operand::Const(self.c_unit)
            }
            Checked::Error => Operand::Const(self.c_unit),
            _ => {
                self.error(
                    DiagnosticKind::ConditionNotBool("a numeric constant".to_string()),
                    at,
                );
                Operand::Const(self.c_unit)
            }
        }
    }

    fn jump_stmt(&mut self, blk: BlockId, node: NodeId) {
        let (stmt, word) = match self.tree.kind(node) {
            NodeKind::BreakStmt => (Stmt::Break, "break"),
            _ => (Stmt::Continue, "continue"),
        };
        if self.cur().loop_depth == 0 {
            self.error(DiagnosticKind::JumpOutsideLoop(word), node);
            return;
        }
        self.emit(blk, stmt, node);
    }

    fn return_stmt(&mut self, blk: BlockId, node: NodeId) {
        let value = cst::expr_children(self.tree, node).first().copied();
        let known = self.cur().ret_known;
        let ret = self.cur().ret;
        match value {
            Some(value) => {
                let expect = if known { Some(ret) } else { None };
                let checked = self.expr(blk, value, expect);
                let (operand, ty) = self.pin(checked, expect, value);
                if !known {
                    // §functions: a lambda with no context to take a return
                    // type from adopts its first `return`.
                    let ctx = self.cur_mut();
                    ctx.ret = ty;
                    ctx.ret_known = true;
                }
                self.emit(blk, Stmt::Return(Some(operand)), node);
            }
            None => {
                if known && !self.program.types.compatible(ret, self.t_unit) {
                    let expected = self.type_name(ret);
                    self.error(
                        DiagnosticKind::TypeMismatch {
                            expected,
                            found: "()".to_string(),
                        },
                        node,
                    );
                }
                self.emit(blk, Stmt::Return(None), node);
            }
        }
    }
}

/// Drops slots nothing mentions, and renumbers what is left.
///
/// [`Lowerer::assign_into`] retargets the instruction that filled a temporary
/// rather than emitting a copy, which leaves the temporary defined nowhere and
/// read nowhere. It cannot always be popped on the spot — by then a later slot
/// may already have been allocated — so the sweep happens once, here.
///
/// Parameters and captures are never dropped: their positions *are* the calling
/// convention, and an unused parameter is still passed. A slot that is only
/// ever assigned to counts as mentioned, because the instruction assigning it
/// may be a call that has to happen anyway.
fn compact_slots(ctx: &mut FuncCtx) {
    let pinned = ctx
        .slots
        .iter()
        .filter(|slot| matches!(slot.kind, SlotKind::Param | SlotKind::Capture))
        .count();

    let mut used = vec![false; ctx.slots.len()];
    for slot in used.iter_mut().take(pinned) {
        *slot = true;
    }
    for block in &ctx.blocks {
        for stmt in &block.stmts {
            visit_slots(stmt, &mut |slot| used[slot.index()] = true);
        }
    }
    if used.iter().all(|used| *used) {
        return;
    }

    let mut remap = vec![Slot(0); ctx.slots.len()];
    let mut next = 0;
    for (index, used) in used.iter().enumerate() {
        if *used {
            remap[index] = Slot(next);
            next += 1;
        }
    }

    let mut kept = Vec::with_capacity(next as usize);
    for (slot, used) in std::mem::take(&mut ctx.slots).into_iter().zip(&used) {
        if *used {
            kept.push(slot);
        }
    }
    ctx.slots = kept;

    for block in &mut ctx.blocks {
        for stmt in &mut block.stmts {
            visit_slots_mut(stmt, &mut |slot| *slot = remap[slot.index()]);
        }
    }
}

/// The rvalues a statement carries. Only two kinds hold one — everything else
/// takes operands, which are atomic by core IR's second invariant.
fn each_rvalue(stmt: &Stmt, f: &mut impl FnMut(&Rvalue)) {
    match stmt {
        Stmt::Assign { rvalue, .. } | Stmt::Drop(rvalue) => f(rvalue),
        _ => {}
    }
}

/// Which globals each function may read, following direct calls to a fixed
/// point.
///
/// A call through a function *value* is answered with the union over every
/// function whose value is taken anywhere — the smallest sound answer
/// available without a points-to analysis, and one that costs nothing in a
/// program that never stores a function.
fn global_reads(program: &Program) -> (Vec<BTreeSet<GlobalId>>, BTreeSet<GlobalId>) {
    let count = program.funcs.len();
    let mut reads = vec![BTreeSet::new(); count];
    let mut calls: Vec<HashSet<usize>> = vec![HashSet::new(); count];
    let mut indirect = vec![false; count];
    let mut escaping: HashSet<usize> = HashSet::new();

    for (index, func) in program.funcs.iter().enumerate() {
        for block in &func.blocks {
            for stmt in &block.stmts {
                each_rvalue(stmt, &mut |rvalue| match rvalue {
                    Rvalue::GlobalGet(id) => {
                        reads[index].insert(*id);
                    }
                    Rvalue::Call { func, .. } => {
                        calls[index].insert(func.index());
                    }
                    Rvalue::CallIndirect { .. } => indirect[index] = true,
                    Rvalue::MakeClosure { func, .. } => {
                        escaping.insert(func.index());
                    }
                    _ => {}
                });
            }
        }
    }

    loop {
        let union: BTreeSet<GlobalId> = escaping
            .iter()
            .flat_map(|func| reads[*func].iter().copied())
            .collect();
        let mut changed = false;
        for index in 0..count {
            let mut extra: BTreeSet<GlobalId> = BTreeSet::new();
            for callee in &calls[index] {
                extra.extend(reads[*callee].iter().copied());
            }
            if indirect[index] {
                extra.extend(union.iter().copied());
            }
            for id in extra {
                changed |= reads[index].insert(id);
            }
        }
        if !changed {
            let union = escaping
                .iter()
                .flat_map(|func| reads[*func].iter().copied())
                .collect();
            return (reads, union);
        }
    }
}

/// Every read of a top-level binding the entry function reaches before that
/// binding's `let` has run.
///
/// Walking the entry function in execution order is enough because a global is
/// initialized by a *top-level* statement, and those are all in its body
/// block; nested blocks are descended for the calls they contain.
fn init_problems(program: &Program, entry: FuncId) -> Vec<(GlobalId, Option<FuncId>, CstId)> {
    let (reads, indirect) = global_reads(program);
    let func = program.func(entry);
    let mut initialized = BTreeSet::new();
    let mut out = Vec::new();
    walk_init(
        func,
        func.body(),
        &reads,
        &indirect,
        &mut initialized,
        &mut out,
    );
    out
}

fn walk_init(
    func: &Func,
    block: BlockId,
    reads: &[BTreeSet<GlobalId>],
    indirect: &BTreeSet<GlobalId>,
    initialized: &mut BTreeSet<GlobalId>,
    out: &mut Vec<(GlobalId, Option<FuncId>, CstId)>,
) {
    let body = func.block(block);
    for (stmt, at) in body.stmts.iter().zip(&body.spans) {
        // Rvalues first: a `let`'s initializer is computed by the statements
        // above the store that binds it, so the store is what marks the
        // binding live and it cannot help the call that preceded it.
        each_rvalue(stmt, &mut |rvalue| {
            let missing = match rvalue {
                Rvalue::GlobalGet(id) if !initialized.contains(id) => Some((*id, None)),
                Rvalue::Call { func: callee, .. } => reads[callee.index()]
                    .iter()
                    .find(|id| !initialized.contains(id))
                    .map(|id| (*id, Some(*callee))),
                Rvalue::CallIndirect { .. } => indirect
                    .iter()
                    .find(|id| !initialized.contains(id))
                    .map(|id| (*id, None)),
                _ => None,
            };
            if let Some((global, via)) = missing {
                out.push((global, via, *at));
            }
        });

        match stmt {
            Stmt::Store {
                place: Place::Global(id),
                ..
            } => {
                initialized.insert(*id);
            }
            Stmt::If { then_, else_, .. } => {
                walk_init(func, *then_, reads, indirect, initialized, out);
                walk_init(func, *else_, reads, indirect, initialized, out);
            }
            Stmt::While { header, body, .. } => {
                walk_init(func, *header, reads, indirect, initialized, out);
                walk_init(func, *body, reads, indirect, initialized, out);
            }
            _ => {}
        }
    }
}

fn visit_slots(stmt: &Stmt, f: &mut impl FnMut(Slot)) {
    let operand = |operand: &Operand, f: &mut dyn FnMut(Slot)| {
        if let Operand::Slot(slot) = operand {
            f(*slot);
        }
    };
    match stmt {
        Stmt::Assign { dst, rvalue } => {
            f(*dst);
            visit_rvalue(rvalue, &mut |slot| f(slot));
        }
        Stmt::Store { place, value } => {
            match place {
                Place::Cell(slot) => f(*slot),
                Place::Field { base, .. } => f(*base),
                Place::Global(_) => {}
            }
            operand(value, f);
        }
        Stmt::If { cond, .. } => operand(cond, f),
        Stmt::While { cond, .. } => operand(cond, f),
        Stmt::Return(Some(value)) => operand(value, f),
        Stmt::Drop(rvalue) => visit_rvalue(rvalue, f),
        Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
    }
}

fn visit_rvalue(rvalue: &Rvalue, f: &mut impl FnMut(Slot)) {
    let mut operand = |operand: &Operand| {
        if let Operand::Slot(slot) = operand {
            f(*slot);
        }
    };
    match rvalue {
        Rvalue::Use(value)
        | Rvalue::Unary(_, value)
        | Rvalue::Cast(value)
        | Rvalue::MakeCell(value)
        | Rvalue::CellGet(value)
        | Rvalue::Field { base: value, .. } => operand(value),
        Rvalue::Binary(_, left, right) => {
            operand(left);
            operand(right);
        }
        Rvalue::Call { args, .. } | Rvalue::MakeStruct(args) => args.iter().for_each(operand),
        Rvalue::CallIndirect { callee, args } => {
            operand(callee);
            args.iter().for_each(operand);
        }
        Rvalue::MakeClosure { captures, .. } => captures.iter().for_each(operand),
        // Storage, not a slot.
        Rvalue::GlobalGet(_) => {}
    }
}

fn visit_slots_mut(stmt: &mut Stmt, f: &mut impl FnMut(&mut Slot)) {
    fn operand(operand: &mut Operand, f: &mut impl FnMut(&mut Slot)) {
        if let Operand::Slot(slot) = operand {
            f(slot);
        }
    }
    match stmt {
        Stmt::Assign { dst, rvalue } => {
            f(dst);
            visit_rvalue_mut(rvalue, f);
        }
        Stmt::Store { place, value } => {
            match place {
                Place::Cell(slot) => f(slot),
                Place::Field { base, .. } => f(base),
                Place::Global(_) => {}
            }
            operand(value, f);
        }
        Stmt::If { cond, .. } => operand(cond, f),
        Stmt::While { cond, .. } => operand(cond, f),
        Stmt::Return(Some(value)) => operand(value, f),
        Stmt::Drop(rvalue) => visit_rvalue_mut(rvalue, f),
        Stmt::Break | Stmt::Continue | Stmt::Return(None) => {}
    }
}

fn visit_rvalue_mut(rvalue: &mut Rvalue, f: &mut impl FnMut(&mut Slot)) {
    fn operand(operand: &mut Operand, f: &mut impl FnMut(&mut Slot)) {
        if let Operand::Slot(slot) = operand {
            f(slot);
        }
    }
    match rvalue {
        Rvalue::Use(value)
        | Rvalue::Unary(_, value)
        | Rvalue::Cast(value)
        | Rvalue::MakeCell(value)
        | Rvalue::CellGet(value)
        | Rvalue::Field { base: value, .. } => operand(value, f),
        Rvalue::Binary(_, left, right) => {
            operand(left, f);
            operand(right, f);
        }
        Rvalue::Call { args, .. } | Rvalue::MakeStruct(args) => {
            args.iter_mut().for_each(|arg| operand(arg, f))
        }
        Rvalue::CallIndirect { callee, args } => {
            operand(callee, f);
            args.iter_mut().for_each(|arg| operand(arg, f));
        }
        Rvalue::MakeClosure { captures, .. } => captures.iter_mut().for_each(|arg| operand(arg, f)),
        Rvalue::GlobalGet(_) => {}
    }
}

#[derive(Clone)]
struct ParamInfo {
    name: String,
    ty: TypeId,
    mutable: bool,
}

// ---- Expressions -----------------------------------------------------------

impl Lowerer<'_> {
    fn expr(&mut self, blk: BlockId, node: NodeId, expect: Option<TypeId>) -> Checked {
        match self.tree.kind(node) {
            NodeKind::LiteralExpr => self.literal(node),
            NodeKind::NameExpr => self.name_expr(blk, node),
            NodeKind::ParenExpr => match cst::expr_children(self.tree, node).first() {
                // §expressions: grouping and nothing else — it does not change
                // a value's type, meaning, or evaluation.
                Some(&inner) => self.expr(blk, inner, expect),
                None => Checked::Error,
            },
            NodeKind::UnitExpr => Checked::Val(Operand::Const(self.c_unit), self.t_unit),
            NodeKind::BlockExpr => self.block_into(blk, node, expect),
            NodeKind::IfExpr => self.if_expr(blk, node, expect),
            NodeKind::UnaryExpr => self.unary(blk, node, expect),
            NodeKind::BinaryExpr => self.binary(blk, node, expect),
            NodeKind::CastExpr => self.cast(blk, node),
            NodeKind::CallExpr => match self.call(blk, node) {
                Some((rvalue, ty)) => self.emit_temp(blk, rvalue, ty, node),
                None => Checked::Error,
            },
            NodeKind::FieldExpr => match self.field_access(blk, node) {
                Some((base, field, ty)) => {
                    self.emit_temp(blk, Rvalue::Field { base, field }, ty, node)
                }
                None => Checked::Error,
            },
            NodeKind::StructLitExpr => self.struct_lit(blk, node),
            NodeKind::LambdaExpr => self.lambda(blk, node, expect),
            NodeKind::IndexExpr => {
                // §types: nothing but `Str` is indexable until §generics
                // brings collections, and what `Str` indexing returns is still
                // open.
                self.error(DiagnosticKind::Unsupported("indexing"), node);
                Checked::Error
            }
            NodeKind::MethodCallExpr => {
                // §objects is unstarted, and §expressions hands it the
                // question of whether `.` on a reference is a local call or a
                // message send.
                self.error(DiagnosticKind::Unsupported("method calls"), node);
                Checked::Error
            }
            NodeKind::ComptimeExpr => {
                // §comptime is decided and unbuilt. The token parses so that
                // the shape of these programs is settled and the language
                // server has something to colour; evaluating one needs `Type`
                // in the value domain and the instantiation cache, neither of
                // which exists. The inner expression is still walked, so a
                // mistake inside it is reported too.
                self.error(DiagnosticKind::Unsupported("comptime"), node);
                if let Some(&inner) = cst::expr_children(self.tree, node).first() {
                    self.expr(blk, inner, expect);
                }
                Checked::Error
            }
            // The parser reported it and kept the bytes. Nothing to add.
            NodeKind::Error => Checked::Error,
            _ => Checked::Error,
        }
    }

    fn literal(&mut self, node: NodeId) -> Checked {
        let Some(token) = self.tree.children(node).find_map(|child| match child {
            parser::Child::Token(token) if !token.is_trivia() => Some(token),
            _ => None,
        }) else {
            return Checked::Error;
        };
        let Some(value) = tokenizer::literal_value(token, self.source) else {
            return Checked::Error;
        };
        match value {
            Literal::Bool(value) => {
                let id = self.program.consts.add(Const::Bool(value));
                Checked::Val(Operand::Const(id), self.t_bool)
            }
            Literal::Char(value) => {
                let id = self.program.consts.add(Const::Char(value));
                Checked::Val(Operand::Const(id), self.t_char)
            }
            Literal::Str(value) => {
                let id = self.program.consts.add(Const::Str(value.into()));
                Checked::Val(Operand::Const(id), self.t_str)
            }
            // §lexical: an unsuffixed literal has no type. It is a
            // mathematical integer that acquires one where it becomes a
            // runtime value.
            Literal::Int {
                value,
                suffix: None,
            } => Checked::Int(value as i128),
            Literal::Int {
                value,
                suffix: Some(suffix),
            } => {
                let ty = self.numeric_type(suffix);
                let value = value as i128;
                // §lexical: an integer may carry a float suffix — `1f64` is a
                // float.
                if self.program.types.is_float(ty) {
                    return Checked::Val(self.float_const(value as f64, ty), ty);
                }
                if !self.fits(value, ty) {
                    let name = self.type_name(ty);
                    self.error(
                        DiagnosticKind::ConstantOutOfRange {
                            value: value.to_string(),
                            ty: name,
                        },
                        node,
                    );
                    return Checked::Error;
                }
                Checked::Val(self.int_const(value, ty), ty)
            }
            Literal::Float {
                value,
                suffix: None,
            } => Checked::Float(value),
            Literal::Float {
                value,
                suffix: Some(suffix),
            } => {
                let ty = self.numeric_type(suffix);
                Checked::Val(self.float_const(value, ty), ty)
            }
        }
    }

    fn numeric_type(&mut self, suffix: NumericType) -> TypeId {
        let def = match suffix {
            NumericType::U8 => TypeDef::Int {
                signed: false,
                bits: 8,
            },
            NumericType::U16 => TypeDef::Int {
                signed: false,
                bits: 16,
            },
            NumericType::U32 => TypeDef::Int {
                signed: false,
                bits: 32,
            },
            NumericType::U64 => TypeDef::Int {
                signed: false,
                bits: 64,
            },
            NumericType::S8 => TypeDef::Int {
                signed: true,
                bits: 8,
            },
            NumericType::S16 => TypeDef::Int {
                signed: true,
                bits: 16,
            },
            NumericType::S32 => TypeDef::Int {
                signed: true,
                bits: 32,
            },
            NumericType::S64 => TypeDef::Int {
                signed: true,
                bits: 64,
            },
            NumericType::F32 => TypeDef::Float { bits: 32 },
            NumericType::F64 => TypeDef::Float { bits: 64 },
        };
        self.program.types.intern(def)
    }

    fn name_expr(&mut self, blk: BlockId, node: NodeId) -> Checked {
        let Some((name, span)) = cst::name(self.tree, self.source, node) else {
            return Checked::Error;
        };
        let Some((owner, binding)) = self.resolve(&name) else {
            self.error_at(DiagnosticKind::UnknownName(name), span);
            return Checked::Error;
        };
        let current = self.funcs.len() - 1;

        match binding {
            Binding::Value { slot, ty, cell, .. } => {
                if owner != current {
                    // §functions: a nested `fn` is scoped to its block and
                    // captures nothing. A lambda's captures are already slots
                    // of its own, put there before its body was walked, so
                    // reaching here means the enclosing function is a `fn`.
                    self.error_at(DiagnosticKind::FnCapturesNothing(name), span);
                    return Checked::Error;
                }
                if cell {
                    self.emit_temp(blk, Rvalue::CellGet(Operand::Slot(slot)), ty, node)
                } else {
                    Checked::Val(Operand::Slot(slot), ty)
                }
            }
            // §statements: a global outlives every frame, so reading one is
            // not a capture and the owner check above does not apply to it.
            Binding::Global { global, ty, .. } => {
                self.emit_temp(blk, Rvalue::GlobalGet(global), ty, node)
            }
            // A constant needs no capture: it is not storage, it is a value
            // known before either function runs.
            Binding::Const(value) => value,
            // §functions: a function is a value. Every function value is a
            // closure, and a plain `fn` has an empty environment.
            Binding::Func { id, ty } => self.emit_temp(
                blk,
                Rvalue::MakeClosure {
                    func: id,
                    captures: Vec::new(),
                },
                ty,
                node,
            ),
            Binding::Type(_) => {
                self.error_at(DiagnosticKind::NotAValue(name), span);
                Checked::Error
            }
        }
    }

    fn if_expr(&mut self, blk: BlockId, node: NodeId, expect: Option<TypeId>) -> Checked {
        let children = cst::nodes(self.tree, node);
        let Some(&cond_node) = children.first() else {
            return Checked::Error;
        };
        let then_node = children.get(1).copied();
        let else_node = children.get(2).copied();

        let checked = self.expr(blk, cond_node, Some(self.t_bool));
        let cond = self.condition(checked, cond_node);

        // §expressions: with no `else` the type is unit, so it can be a
        // statement but not a value.
        let wants_value = else_node.is_some();
        let arm_expect = if wants_value { expect } else { None };

        let then_ = self.new_block();
        let then_value = match then_node {
            Some(then_node) => self.block_into(then_, then_node, arm_expect),
            None => Checked::Error,
        };

        let else_ = self.new_block();
        let else_value = match else_node {
            Some(else_node) if self.tree.kind(else_node) == NodeKind::BlockExpr => {
                self.block_into(else_, else_node, arm_expect)
            }
            // `else if` is `else` followed by another `if` (§control), so the
            // chain nests rather than flattening.
            Some(else_node) => self.expr(else_, else_node, arm_expect),
            None => Checked::Val(Operand::Const(self.c_unit), self.t_unit),
        };

        if !wants_value {
            if let Some(want) = expect
                && !self.program.types.compatible(want, self.t_unit)
            {
                let expected = self.type_name(want);
                self.error(
                    DiagnosticKind::TypeMismatch {
                        expected,
                        found: "()".to_string(),
                    },
                    node,
                );
            }
            self.emit(blk, Stmt::If { cond, then_, else_ }, node);
            return Checked::Val(Operand::Const(self.c_unit), self.t_unit);
        }

        // Both arms must agree. The `if`'s value is a slot both arms assign
        // into, which is what a phi node would otherwise be for.
        let (then_op, ty) = self.pin(then_value, expect, then_node.unwrap_or(node));
        let (else_op, _) = self.pin(else_value, Some(ty), else_node.unwrap_or(node));

        if self.program.types.compatible(ty, self.t_unit) {
            self.emit(blk, Stmt::If { cond, then_, else_ }, node);
            return Checked::Val(Operand::Const(self.c_unit), self.t_unit);
        }

        let dst = self.temp(ty);
        self.assign_into(then_, dst, then_op, then_node.unwrap_or(node));
        self.assign_into(else_, dst, else_op, else_node.unwrap_or(node));
        self.emit(blk, Stmt::If { cond, then_, else_ }, node);
        Checked::Val(Operand::Slot(dst), ty)
    }

    fn operator(&self, node: NodeId) -> Option<TokenKind> {
        self.tree.children(node).find_map(|child| match child {
            parser::Child::Token(token) if !token.is_trivia() => Some(token.kind),
            _ => None,
        })
    }

    fn unary(&mut self, blk: BlockId, node: NodeId, expect: Option<TypeId>) -> Checked {
        let Some(&operand_node) = cst::expr_children(self.tree, node).first() else {
            return Checked::Error;
        };
        let op = self.operator(node);
        let value = self.expr(blk, operand_node, expect);

        match op {
            Some(TokenKind::Minus) => match value {
                // §lexical: unary `-` is an ordinary operation on constants,
                // which is why `-1` needs no rule of its own in the lexer.
                Checked::Int(v) => Checked::Int(-v),
                Checked::Float(v) => Checked::Float(-v),
                Checked::Error => Checked::Error,
                Checked::Val(operand, ty) => {
                    if self.program.types.is_error(ty) {
                        return Checked::Error;
                    }
                    // §expressions: negating an unsigned value has no
                    // representable result but zero, so it is a type error
                    // rather than a trap — the error arrives earlier and says
                    // more.
                    if !self.program.types.is_signed_or_float(ty) {
                        let name = self.type_name(ty);
                        let kind = if self.program.types.is_integer(ty) {
                            DiagnosticKind::NegateUnsigned(name)
                        } else {
                            DiagnosticKind::OperatorNotDefined {
                                op: "-".to_string(),
                                ty: name,
                            }
                        };
                        self.error(kind, node);
                        return Checked::Error;
                    }
                    self.emit_temp(blk, Rvalue::Unary(UnOp::Neg, operand), ty, node)
                }
            },
            Some(TokenKind::Bang) => {
                let (operand, ty) = self.pin(value, Some(self.t_bool), operand_node);
                if self.program.types.is_error(ty) {
                    return Checked::Error;
                }
                self.emit_temp(blk, Rvalue::Unary(UnOp::Not, operand), self.t_bool, node)
            }
            _ => Checked::Error,
        }
    }

    fn binary(&mut self, blk: BlockId, node: NodeId, expect: Option<TypeId>) -> Checked {
        let children = cst::expr_children(self.tree, node);
        let (Some(&lhs_node), Some(&rhs_node)) = (children.first(), children.get(1)) else {
            return Checked::Error;
        };
        let Some(token) = self.operator(node) else {
            return Checked::Error;
        };

        // §expressions: `&&` and `||` short-circuit, which makes them control
        // flow wearing an operator's clothes. They lower to `if`, so neither
        // back end implements laziness and `BinOp` has no entry for them.
        if matches!(token, TokenKind::AmpAmp | TokenKind::PipePipe) {
            return self.short_circuit(blk, node, lhs_node, rhs_node, token);
        }

        let Some(op) = binop(token) else {
            return Checked::Error;
        };

        // §expressions: operands evaluate left then right. In ANF that is the
        // order of the statements below, so it is the shape of the data rather
        // than a rule two back ends each have to remember.
        let hint = if op.is_comparison() { None } else { expect };
        let left = self.expr(blk, lhs_node, hint);
        let hint = self.known_type(&left).or(hint);
        let right = self.expr(blk, rhs_node, hint);

        if matches!(left, Checked::Error) || matches!(right, Checked::Error) {
            return Checked::Error;
        }

        // §lexical: arithmetic over unpinned constants happens with unbounded
        // precision and the *result* is what gets typed, so an intermediate
        // can never overflow.
        if let Some(folded) = self.fold(op, &left, &right, node) {
            return folded;
        }

        // A hint pins an unpinned constant; it is not a check on a value that
        // already has a type. Passing it to both would report "expected `u32`,
        // found `u64`" and bury §lexical's actual rule.
        let known = self.known_type(&left).or_else(|| self.known_type(&right));
        let (left_op, left_ty) = self.pin_operand(left, known, lhs_node);
        let (right_op, right_ty) = self.pin_operand(right, known, rhs_node);

        if self.program.types.is_error(left_ty) || self.program.types.is_error(right_ty) {
            return Checked::Error;
        }
        // §lexical: no implicit conversion between pinned types. `u64 + s64`
        // is an error, and so is `u32 + u64` — the alternative is C's
        // promotion lattice, where mixed comparison silently does the wrong
        // thing.
        if left_ty != right_ty {
            let (left, right) = (self.type_name(left_ty), self.type_name(right_ty));
            self.error(
                DiagnosticKind::MixedOperands {
                    op: op.spelling().to_string(),
                    left,
                    right,
                },
                node,
            );
            return Checked::Error;
        }
        if !self.defined_on(op, left_ty) {
            let ty = self.type_name(left_ty);
            self.error(
                DiagnosticKind::OperatorNotDefined {
                    op: op.spelling().to_string(),
                    ty,
                },
                node,
            );
            return Checked::Error;
        }

        let ty = if op.is_comparison() {
            self.t_bool
        } else {
            left_ty
        };
        if let Some(folded) = self.fold_pinned(op, left_op, right_op, ty, node) {
            return folded;
        }
        self.emit_temp(blk, Rvalue::Binary(op, left_op, right_op), ty, node)
    }

    /// Which operators §expressions defines on which types. They are built-in
    /// and closed until §types brings traits and operator overloading.
    fn defined_on(&self, op: BinOp, ty: TypeId) -> bool {
        let types = &self.program.types;
        match op {
            // §expressions: `+` also concatenates strings.
            BinOp::Add => types.is_numeric(ty) || matches!(types.get(ty), TypeDef::Str),
            BinOp::Sub | BinOp::Mul | BinOp::Div => types.is_numeric(ty),
            BinOp::Rem => types.is_numeric(ty),
            // §expressions: structural for values, and cross-type comparison
            // does not exist — that part is checked by the operand types
            // agreeing.
            BinOp::Eq | BinOp::Ne => !matches!(types.get(ty), TypeDef::Fn { .. }),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => types.is_ordered(ty),
        }
    }

    /// Constant folding over §lexical's unpinned constants.
    ///
    /// §expressions: "constant expressions are checked at compile time rather
    /// than trapping", so overflow and division by zero are diagnostics here
    /// rather than runtime traps.
    fn fold(
        &mut self,
        op: BinOp,
        left: &Checked,
        right: &Checked,
        node: NodeId,
    ) -> Option<Checked> {
        match (left, right) {
            (Checked::Int(a), Checked::Int(b)) => {
                let (a, b) = (*a, *b);
                Some(match op {
                    BinOp::Add => match a.checked_add(b) {
                        Some(value) => Checked::Int(value),
                        None => self.constant_overflow(node),
                    },
                    BinOp::Sub => match a.checked_sub(b) {
                        Some(value) => Checked::Int(value),
                        None => self.constant_overflow(node),
                    },
                    BinOp::Mul => match a.checked_mul(b) {
                        Some(value) => Checked::Int(value),
                        None => self.constant_overflow(node),
                    },
                    // §expressions: division truncates toward zero and the
                    // remainder takes the dividend's sign, which is what
                    // `i128` already does.
                    BinOp::Div | BinOp::Rem if b == 0 => {
                        self.error(DiagnosticKind::ConstantDivisionByZero, node);
                        Checked::Error
                    }
                    BinOp::Div => Checked::Int(a / b),
                    BinOp::Rem => Checked::Int(a % b),
                    _ => self.bool_const(compare_ord(op, a.cmp(&b))),
                })
            }
            (Checked::Float(a), Checked::Float(b)) => {
                let (a, b) = (*a, *b);
                Some(match op {
                    BinOp::Add => Checked::Float(a + b),
                    BinOp::Sub => Checked::Float(a - b),
                    BinOp::Mul => Checked::Float(a * b),
                    BinOp::Div => Checked::Float(a / b),
                    BinOp::Rem => Checked::Float(a % b),
                    // IEEE-754 (§expressions): every ordering against NaN is
                    // false, and `NaN != NaN`.
                    BinOp::Eq => self.bool_const(a == b),
                    BinOp::Ne => self.bool_const(a != b),
                    BinOp::Lt => self.bool_const(a < b),
                    BinOp::Le => self.bool_const(a <= b),
                    BinOp::Gt => self.bool_const(a > b),
                    BinOp::Ge => self.bool_const(a >= b),
                })
            }
            // §lexical: integer and float constants do not mix.
            (Checked::Int(_), Checked::Float(_)) | (Checked::Float(_), Checked::Int(_)) => {
                self.error(DiagnosticKind::MixedConstantKinds, node);
                Some(Checked::Error)
            }
            _ => None,
        }
    }

    /// [`Lowerer::pin`], except that a hint is only applied to an unpinned
    /// constant. A value that already has a type keeps it, so the caller can
    /// report the disagreement in its own words.
    fn pin_operand(
        &mut self,
        value: Checked,
        hint: Option<TypeId>,
        at: NodeId,
    ) -> (Operand, TypeId) {
        match value {
            Checked::Val(..) => self.pin(value, None, at),
            _ => self.pin(value, hint, at),
        }
    }

    /// §expressions: "Constant expressions are checked at compile time rather
    /// than trapping. Anything that would trap at runtime — overflow, division
    /// by zero — is a compile error when all operands are constants."
    ///
    /// [`Lowerer::fold`] covers §lexical's *unpinned* constants, which have no
    /// width to overflow. This covers the pinned ones, where `255u8 + 1` has
    /// to be an error rather than `0`.
    fn fold_pinned(
        &mut self,
        op: BinOp,
        left: Operand,
        right: Operand,
        ty: TypeId,
        node: NodeId,
    ) -> Option<Checked> {
        let (Operand::Const(left), Operand::Const(right)) = (left, right) else {
            return None;
        };
        let (left, right) = (
            self.program.consts.get(left).clone(),
            self.program.consts.get(right).clone(),
        );

        match (left, right) {
            (
                Const::Int {
                    ty: int_ty,
                    bits: a,
                },
                Const::Int { bits: b, .. },
            ) => {
                let (a, b) = (
                    decode_int(&self.program.types, int_ty, a)?,
                    decode_int(&self.program.types, int_ty, b)?,
                );
                if op.is_comparison() {
                    return Some(self.bool_const(compare_ord(op, a.cmp(&b))));
                }
                let value = match op {
                    BinOp::Add => a.checked_add(b),
                    BinOp::Sub => a.checked_sub(b),
                    BinOp::Mul => a.checked_mul(b),
                    BinOp::Div | BinOp::Rem if b == 0 => {
                        self.error(DiagnosticKind::ConstantDivisionByZero, node);
                        return Some(Checked::Error);
                    }
                    // §expressions: division truncates toward zero and the
                    // remainder takes the dividend's sign.
                    BinOp::Div => Some(a / b),
                    BinOp::Rem => Some(a % b),
                    _ => return None,
                };
                // §lexical and §expressions: once pinned, exceeding the type's
                // range is an error rather than a wrap — and as a constant it
                // is an error now rather than a trap that never runs.
                match value.filter(|value| self.fits(*value, int_ty)) {
                    Some(value) => Some(Checked::Val(self.int_const(value, int_ty), int_ty)),
                    None => {
                        self.error(DiagnosticKind::ConstantOverflow, node);
                        Some(Checked::Error)
                    }
                }
            }
            (
                Const::Float {
                    ty: float_ty,
                    bits: a,
                },
                Const::Float { bits: b, .. },
            ) => {
                let (a, b) = (f64::from_bits(a), f64::from_bits(b));
                if op.is_comparison() {
                    // IEEE-754 (§expressions), so every ordering against NaN
                    // is false.
                    return Some(self.bool_const(match op {
                        BinOp::Eq => a == b,
                        BinOp::Ne => a != b,
                        BinOp::Lt => a < b,
                        BinOp::Le => a <= b,
                        BinOp::Gt => a > b,
                        _ => a >= b,
                    }));
                }
                let value = match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div => a / b,
                    BinOp::Rem => a % b,
                    _ => return None,
                };
                Some(Checked::Val(self.float_const(value, float_ty), float_ty))
            }
            // §expressions: `+` concatenates, and `==` and ordering compare
            // bytes.
            (Const::Str(a), Const::Str(b)) => Some(match op {
                BinOp::Add => {
                    let joined: Rc<str> = format!("{a}{b}").into();
                    let id = self.program.consts.add(Const::Str(joined));
                    Checked::Val(Operand::Const(id), ty)
                }
                _ => self.bool_const(compare_ord(op, a.cmp(&b))),
            }),
            (Const::Bool(a), Const::Bool(b)) if op.is_comparison() => {
                Some(self.bool_const(compare_ord(op, a.cmp(&b))))
            }
            (Const::Char(a), Const::Char(b)) if op.is_comparison() => {
                Some(self.bool_const(compare_ord(op, a.cmp(&b))))
            }
            _ => None,
        }
    }

    fn constant_overflow(&mut self, node: NodeId) -> Checked {
        self.error(DiagnosticKind::ConstantOverflow, node);
        Checked::Error
    }

    fn bool_const(&mut self, value: bool) -> Checked {
        let id = self.program.consts.add(Const::Bool(value));
        Checked::Val(Operand::Const(id), self.t_bool)
    }

    fn short_circuit(
        &mut self,
        blk: BlockId,
        node: NodeId,
        lhs_node: NodeId,
        rhs_node: NodeId,
        token: TokenKind,
    ) -> Checked {
        let dst = self.temp(self.t_bool);
        let left = self.expr(blk, lhs_node, Some(self.t_bool));
        let left = self.condition(left, lhs_node);
        self.assign_into(blk, dst, left, lhs_node);

        let then_ = self.new_block();
        let else_ = self.new_block();
        // `&&` evaluates the right operand only when the left was true; `||`
        // only when it was false. The other arm is empty because `dst` already
        // holds the answer.
        let arm = if token == TokenKind::AmpAmp {
            then_
        } else {
            else_
        };
        let right = self.expr(arm, rhs_node, Some(self.t_bool));
        let right = self.condition(right, rhs_node);
        self.assign_into(arm, dst, right, rhs_node);

        self.emit(
            blk,
            Stmt::If {
                cond: Operand::Slot(dst),
                then_,
                else_,
            },
            node,
        );
        Checked::Val(Operand::Slot(dst), self.t_bool)
    }

    /// `as` over §lexical's unpinned constants, following §expressions'
    /// conversion table: integer to integer is exact or an error, float to
    /// integer truncates toward zero, and integer to float rounds — because
    /// inexactness is inherent to floats and is not an error, where integer
    /// overflow is.
    fn cast_const(&mut self, value: Checked, target: TypeId, node: NodeId) -> Checked {
        let out_of_range = |lowerer: &mut Self, shown: String| {
            let ty = lowerer.type_name(target);
            lowerer.error(
                DiagnosticKind::ConstantOutOfRange { value: shown, ty },
                node,
            );
            Checked::Error
        };
        match value {
            Checked::Int(v) if self.program.types.is_float(target) => {
                Checked::Val(self.float_const(v as f64, target), target)
            }
            Checked::Int(v) if self.fits(v, target) => {
                Checked::Val(self.int_const(v, target), target)
            }
            Checked::Int(v) => out_of_range(self, v.to_string()),
            Checked::Float(v) if self.program.types.is_float(target) => {
                Checked::Val(self.float_const(v, target), target)
            }
            Checked::Float(v) if self.program.types.is_integer(target) => {
                // §expressions: traps on NaN and on the infinities, so as a
                // constant it is refused rather than given a value nobody
                // chose.
                if !v.is_finite() {
                    return out_of_range(self, v.to_string());
                }
                let truncated = v.trunc();
                if truncated < i128::MIN as f64 || truncated > i128::MAX as f64 {
                    return out_of_range(self, v.to_string());
                }
                let truncated = truncated as i128;
                if !self.fits(truncated, target) {
                    return out_of_range(self, v.to_string());
                }
                Checked::Val(self.int_const(truncated, target), target)
            }
            Checked::Float(_) => {
                let to = self.type_name(target);
                self.error(
                    DiagnosticKind::CannotCast {
                        from: "f64".to_string(),
                        to,
                    },
                    node,
                );
                Checked::Error
            }
            other => other,
        }
    }

    fn cast(&mut self, blk: BlockId, node: NodeId) -> Checked {
        let Some(&value_node) = cst::expr_children(self.tree, node).first() else {
            return Checked::Error;
        };
        let target = match cst::type_child(self.tree, node) {
            Some(ty) => self.resolve_type(ty),
            None => return Checked::Error,
        };
        let value = self.expr(blk, value_node, None);

        // §expressions: a conversion that would *trap* at run time is a
        // compile error when every operand is constant. Truncation and
        // rounding are defined behaviour, not traps, so they happen here
        // rather than being refused.
        match value {
            Checked::Int(_) | Checked::Float(_) => return self.cast_const(value, target, node),
            Checked::Error => return Checked::Error,
            Checked::Val(..) => {}
        }

        let (operand, from) = self.pin(value, None, value_node);
        if self.program.types.is_error(from) || self.program.types.is_error(target) {
            return Checked::Error;
        }
        if from == target {
            return Checked::Val(operand, target);
        }
        // Numeric conversions only. §expressions spells out what each one does
        // — integer to integer traps unless representable, float to integer
        // truncates and traps at the edges, integer to float rounds — and all
        // of that is the back ends' to implement from one node.
        if !(self.program.types.is_numeric(from) && self.program.types.is_numeric(target)) {
            let (from, to) = (self.type_name(from), self.type_name(target));
            self.error(DiagnosticKind::CannotCast { from, to }, node);
            return Checked::Error;
        }
        self.emit_temp(blk, Rvalue::Cast(operand), target, node)
    }
}

/// A stored integer constant as its mathematical value: sign-extended from the
/// type's width when the type is signed, and read as-is otherwise.
fn decode_int(types: &Types, ty: TypeId, bits: u64) -> Option<i128> {
    match *types.get(ty) {
        TypeDef::Int {
            signed: true,
            bits: width,
        } => {
            let shift = 64 - width as u32;
            Some((((bits as i64) << shift) >> shift) as i128)
        }
        TypeDef::Int {
            signed: false,
            bits: width,
        } => {
            let mask = if width == 64 {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            Some((bits & mask) as i128)
        }
        _ => None,
    }
}

fn binop(token: TokenKind) -> Option<BinOp> {
    Some(match token {
        TokenKind::Plus => BinOp::Add,
        TokenKind::Minus => BinOp::Sub,
        TokenKind::Star => BinOp::Mul,
        TokenKind::Slash => BinOp::Div,
        TokenKind::Percent => BinOp::Rem,
        TokenKind::EqualTo => BinOp::Eq,
        TokenKind::NotEqualTo => BinOp::Ne,
        TokenKind::LessThan => BinOp::Lt,
        TokenKind::LessThanOrEqualTo => BinOp::Le,
        TokenKind::GreaterThan => BinOp::Gt,
        TokenKind::GreaterThanOrEqualTo => BinOp::Ge,
        _ => return None,
    })
}

fn compare_ord(op: BinOp, ordering: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        BinOp::Eq => ordering == Equal,
        BinOp::Ne => ordering != Equal,
        BinOp::Lt => ordering == Less,
        BinOp::Le => ordering != Greater,
        BinOp::Gt => ordering == Greater,
        BinOp::Ge => ordering != Less,
        _ => false,
    }
}

// ---- Calls, fields, structs ------------------------------------------------

impl Lowerer<'_> {
    /// Returns the rvalue rather than a [`Checked`], because §statements'
    /// expression statement wants to discard it and every other caller wants
    /// it in a temporary.
    fn call(&mut self, blk: BlockId, node: NodeId) -> Option<(Rvalue, TypeId)> {
        let children = cst::nodes(self.tree, node);
        let callee_node = *children.first()?;
        let args: Vec<NodeId> = children
            .iter()
            .find(|child| self.tree.kind(**child) == NodeKind::ArgList)
            .map(|list| cst::nodes(self.tree, *list))
            .unwrap_or_default();

        // A direct call when the callee names a `fn`, an indirect one through
        // any other function value. §functions checks arity and types
        // statically, so there is nothing dynamic left to check at run time.
        let direct = if self.tree.kind(callee_node) == NodeKind::NameExpr {
            cst::name(self.tree, self.source, callee_node)
                .and_then(|(name, _)| self.resolve(&name))
                .and_then(|(_, binding)| match binding {
                    Binding::Func { id, ty } => Some((id, ty)),
                    _ => None,
                })
        } else {
            None
        };

        let (callee, sig) = match direct {
            Some((id, ty)) => (None, Some((id, ty))),
            None => {
                let checked = self.expr(blk, callee_node, None);
                let (operand, ty) = self.pin(checked, None, callee_node);
                if self.program.types.is_error(ty) {
                    return None;
                }
                if self.program.types.fn_sig(ty).is_none() {
                    let name = self.type_name(ty);
                    self.error(DiagnosticKind::NotCallable(name), callee_node);
                    return None;
                }
                (Some((operand, ty)), None)
            }
        };

        let fn_ty = match (&callee, &sig) {
            (_, Some((_, ty))) => *ty,
            (Some((_, ty)), _) => *ty,
            _ => return None,
        };
        let (params, ret) = self.program.types.fn_sig(fn_ty)?;

        if params.len() != args.len() {
            self.error(
                DiagnosticKind::WrongArity {
                    expected: params.len(),
                    found: args.len(),
                },
                node,
            );
        }

        // §functions' `mut` rule is checked only where the callee is known by
        // name. A `fn` type carries no `mut` (§functions gives one no syntax),
        // so a call through a function *value* has nothing to check against —
        // the check travels with the declaration rather than with the type.
        let mut_params = sig
            .and_then(|(id, _)| self.mut_params.get(&id).cloned())
            .unwrap_or_default();

        // §expressions and §functions: arguments evaluate left to right,
        // before the call.
        let mut operands = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            if let Some((name, true)) = mut_params.get(index) {
                let name = name.clone();
                self.check_mut_argument(*arg, &name);
            }
            let expect = params.get(index).copied();
            let checked = self.expr(blk, *arg, expect);
            let (operand, _) = self.pin(checked, expect, *arg);
            operands.push(operand);
        }
        if params.len() != args.len() {
            return None;
        }

        let rvalue = match (callee, sig) {
            (_, Some((func, _))) => Rvalue::Call {
                func,
                args: operands,
            },
            (Some((callee, _)), _) => Rvalue::CallIndirect {
                callee,
                args: operands,
            },
            _ => return None,
        };
        Some((rvalue, ret))
    }

    /// §functions: a `mut` parameter permits the callee to mutate the argument
    /// in place, and §statements is the rule that consumes it — mutating
    /// through a binding requires that binding to be `mut`, at the call site
    /// as everywhere else.
    ///
    /// Nothing is written *back*: §statements' `mut` gates in-place mutation,
    /// so what the caller observes is whatever §types' semantics make
    /// observable through the value it passed. For a struct that is the
    /// mutation itself; for a scalar there is nothing to alias, and the callee
    /// mutates its own slot.
    fn check_mut_argument(&mut self, arg: NodeId, parameter: &str) {
        let Some((root, span)) = self.place_root(arg) else {
            self.error(
                DiagnosticKind::MutArgumentNotAPlace {
                    parameter: parameter.to_string(),
                },
                arg,
            );
            return;
        };
        match self.resolve(&root) {
            // §statements' rule reads the binding, not its storage — a global
            // is `mut` or not exactly as a local is.
            Some((
                _,
                Binding::Value { mutable: true, .. } | Binding::Global { mutable: true, .. },
            )) => {}
            // An unknown name is reported once, when the argument is lowered.
            None => {}
            _ => self.error_at(
                DiagnosticKind::MutArgumentNotMutable {
                    parameter: parameter.to_string(),
                    argument: root,
                },
                span,
            ),
        }
    }

    fn field_access(&mut self, blk: BlockId, node: NodeId) -> Option<(Operand, FieldIdx, TypeId)> {
        let base_node = *cst::nodes(self.tree, node).first()?;
        let (field, span) = cst::name(self.tree, self.source, node)?;

        let checked = self.expr(blk, base_node, None);
        let (base, base_ty) = self.pin(checked, None, base_node);
        if self.program.types.is_error(base_ty) {
            return None;
        }

        let Some(def) = self.program.types.struct_def(base_ty) else {
            let name = self.type_name(base_ty);
            self.error_at(DiagnosticKind::NotAStruct(name), span);
            return None;
        };
        let found = def
            .fields
            .iter()
            .position(|def| self.program.syms.text(def.name) == field)
            .map(|index| (FieldIdx(index as u32), def.fields[index].ty));

        match found {
            Some((index, ty)) => Some((base, index, ty)),
            None => {
                let ty = self.type_name(base_ty);
                self.error_at(DiagnosticKind::UnknownField { ty, field }, span);
                None
            }
        }
    }

    fn struct_lit(&mut self, blk: BlockId, node: NodeId) -> Checked {
        let children = cst::nodes(self.tree, node);
        let Some(&name_node) = children.first() else {
            return Checked::Error;
        };
        let ty = self.resolve_type_name(name_node);
        let Some(def) = self.program.types.struct_def(ty) else {
            if !self.program.types.is_error(ty) {
                let name = self.type_name(ty);
                self.error(DiagnosticKind::NotAStruct(name), name_node);
            }
            return Checked::Error;
        };
        let fields: Vec<(String, TypeId)> = def
            .fields
            .iter()
            .map(|field| (self.program.syms.text(field.name).to_string(), field.ty))
            .collect();

        let inits = children
            .iter()
            .find(|child| self.tree.kind(**child) == NodeKind::FieldInitList)
            .map(|list| cst::nodes(self.tree, *list))
            .unwrap_or_default();

        // §expressions: fields evaluate in *written* order. They are stored in
        // *declaration* order, which is what `MakeStruct` takes — so the two
        // orders are separated here rather than left for a back end to guess.
        let mut given: Vec<Option<Operand>> = vec![None; fields.len()];
        for init in inits {
            let Some((name, span)) = cst::name(self.tree, self.source, init) else {
                continue;
            };
            let Some(index) = fields.iter().position(|(field, _)| *field == name) else {
                let ty = self.type_name(ty);
                self.error_at(DiagnosticKind::UnknownField { ty, field: name }, span);
                continue;
            };
            let value = cst::expr_children(self.tree, init).first().copied();
            let expect = fields[index].1;
            let checked = match value {
                Some(value) => self.expr(blk, value, Some(expect)),
                None => Checked::Error,
            };
            let (operand, _) = self.pin(checked, Some(expect), value.unwrap_or(init));
            if given[index].is_some() {
                self.error_at(DiagnosticKind::DuplicateField(name), span);
                continue;
            }
            given[index] = Some(operand);
        }

        let mut operands = Vec::with_capacity(fields.len());
        let mut complete = true;
        for (index, (name, _)) in fields.iter().enumerate() {
            match given[index] {
                Some(operand) => operands.push(operand),
                None => {
                    self.error(DiagnosticKind::MissingField(name.clone()), node);
                    complete = false;
                }
            }
        }
        if !complete {
            return Checked::Error;
        }

        self.emit_temp(blk, Rvalue::MakeStruct(operands), ty, node)
    }
}

// ---- Functions and lambdas -------------------------------------------------

struct CaptureInfo {
    name: String,
    parent_slot: Slot,
    /// `Cell(T)` when the binding needs one, `T` otherwise.
    storage_ty: TypeId,
    value_ty: TypeId,
    cell: bool,
    mutable: bool,
}

impl Lowerer<'_> {
    /// Lowers the body of a `fn` that [`Lowerer::hoist_fn`] already gave a
    /// signature and an id.
    fn fn_body(&mut self, node: NodeId) {
        let Some((name, _)) = cst::name(self.tree, self.source, node) else {
            return;
        };
        let Some((_, Binding::Func { id, .. })) = self.resolve(&name) else {
            return;
        };
        let Some((params, ret)) = self.signatures.get(&node).cloned() else {
            return;
        };
        let sym = self.program.syms.intern(&name);

        self.reuse_func(id, Some(sym), ret, node);
        self.push_scope(None);
        self.bind_params(&params);

        let body_node = cst::nodes(self.tree, node)
            .into_iter()
            .find(|child| self.tree.kind(*child) == NodeKind::BlockExpr);
        let body = self.new_block();
        let value = match body_node {
            // §functions: the body is a block, so its value is its trailing
            // expression. `return` is for early exit, and a well-shaped
            // function often has none.
            Some(body_node) => self.block_into(body, body_node, Some(ret)),
            None => Checked::Error,
        };
        let (operand, _) = self.pin(value, Some(ret), body_node.unwrap_or(node));
        self.emit_result(body, operand, node);

        self.pop_scope();
        self.end_func();
    }

    fn bind_params(&mut self, params: &[ParamInfo]) {
        for param in params {
            let sym = self.program.syms.intern(&param.name);
            let slot = self.slot(param.ty, Some(sym), SlotKind::Param, param.mutable);
            self.bind(
                param.name.clone(),
                Binding::Value {
                    slot,
                    ty: param.ty,
                    // A parameter is never a cell: §functions gives a lambda
                    // no way to rebind one, since a lambda's own parameters
                    // shadow it and an enclosing function's are captured by
                    // value.
                    cell: false,
                    mutable: param.mutable,
                },
            );
        }
    }

    fn lambda(&mut self, blk: BlockId, node: NodeId, expect: Option<TypeId>) -> Checked {
        let children = cst::nodes(self.tree, node);
        let param_nodes = children
            .iter()
            .find(|child| self.tree.kind(**child) == NodeKind::LambdaParamList)
            .map(|list| cst::nodes(self.tree, *list))
            .unwrap_or_default();
        let body_node = children
            .iter()
            .rev()
            .find(|child| cst::is_expr(self.tree.kind(**child)))
            .copied();

        // §functions: a lambda's parameter and return types come from context,
        // unlike a named `fn`. A `fn` is a declaration others read; a lambda
        // is an argument read in place.
        let expected = expect.and_then(|ty| self.program.types.fn_sig(ty));
        let mut params = Vec::with_capacity(param_nodes.len());
        for (index, param) in param_nodes.iter().enumerate() {
            let name = cst::name(self.tree, self.source, *param)
                .map(|(name, _)| name)
                .unwrap_or_default();
            let annotated = cst::type_child(self.tree, *param).map(|ty| self.resolve_type(ty));
            let from_context = expected
                .as_ref()
                .and_then(|(params, _)| params.get(index).copied());
            let ty = match (annotated, from_context) {
                (Some(annotated), Some(wanted)) => {
                    if !self.program.types.compatible(annotated, wanted) {
                        let (expected, found) = (self.type_name(wanted), self.type_name(annotated));
                        self.error(DiagnosticKind::TypeMismatch { expected, found }, *param);
                    }
                    annotated
                }
                (Some(annotated), None) => annotated,
                (None, Some(wanted)) => wanted,
                (None, None) => {
                    self.error(DiagnosticKind::CannotInferLambda, node);
                    self.t_error
                }
            };
            params.push(ParamInfo {
                name,
                ty,
                mutable: false,
            });
        }

        let captures = self.captures_of(node);
        let ret = expected.as_ref().map(|(_, ret)| *ret);

        let parent = self.funcs.len() - 1;
        let index = self.funcs[parent].lambdas;
        self.funcs[parent].lambdas += 1;
        let label = match self.funcs[parent].name {
            Some(name) => format!("{}.λ{index}", self.program.syms.text(name)),
            None => format!("λ{index}"),
        };
        let sym = self.program.syms.intern(&label);

        let id = self.begin_func(Some(sym), ret.unwrap_or(self.t_error), node);
        self.cur_mut().ret_known = ret.is_some();
        self.push_scope(None);
        self.bind_params(&params);

        // Capture slots sit directly after the parameters, so the calling
        // convention can fill them from the closure environment by position.
        for capture in &captures {
            let sym = self.program.syms.intern(&capture.name);
            let slot = self.slot(
                capture.storage_ty,
                Some(sym),
                SlotKind::Capture,
                capture.mutable,
            );
            self.bind(
                capture.name.clone(),
                Binding::Value {
                    slot,
                    ty: capture.value_ty,
                    cell: capture.cell,
                    mutable: capture.mutable,
                },
            );
        }

        let body = self.new_block();
        let value = match body_node {
            Some(body_node) => self.expr(body, body_node, ret),
            None => Checked::Error,
        };
        let (operand, value_ty) = self.pin(value, ret, body_node.unwrap_or(node));
        if !self.cur().ret_known {
            self.cur_mut().ret = value_ty;
            self.cur_mut().ret_known = true;
        }
        self.emit_result(body, operand, node);

        let ret = self.cur().ret;
        self.pop_scope();
        self.end_func();

        // A lambda whose parameters had nothing to infer from is poison rather
        // than `fn(?) -> ?`, so the one mistake produces the one diagnostic
        // already reported instead of a second at every call site.
        if params
            .iter()
            .any(|param| self.program.types.is_error(param.ty))
            || self.program.types.is_error(ret)
        {
            return Checked::Error;
        }

        let ty = self.program.types.intern(TypeDef::Fn {
            params: params.iter().map(|param| param.ty).collect(),
            ret,
        });
        let operands = captures
            .iter()
            .map(|capture| Operand::Slot(capture.parent_slot))
            .collect();
        self.emit_temp(
            blk,
            Rvalue::MakeClosure {
                func: id,
                captures: operands,
            },
            ty,
            node,
        )
    }

    /// What a lambda captures, resolved in the enclosing scopes before its own
    /// are pushed.
    ///
    /// A name that resolves to a constant, a `fn`, or a type is not captured:
    /// none of them is storage belonging to a frame.
    fn captures_of(&mut self, node: NodeId) -> Vec<CaptureInfo> {
        let current = self.funcs.len() - 1;
        let mut captures = Vec::new();
        for name in scan::free_vars(self.tree, self.source, node) {
            let Some((
                owner,
                Binding::Value {
                    slot,
                    ty,
                    cell,
                    mutable,
                },
            )) = self.resolve(&name)
            else {
                continue;
            };
            if owner == current {
                // Declared in this same function, so the lambda reaches it
                // through the frame it is being built in.
            }
            let storage_ty = if cell {
                self.program.types.intern(TypeDef::Cell(ty))
            } else {
                ty
            };
            captures.push(CaptureInfo {
                name,
                parent_slot: slot,
                storage_ty,
                value_ty: ty,
                cell,
                mutable,
            });
        }
        captures
    }

    fn reuse_func(&mut self, id: FuncId, name: Option<Sym>, ret: TypeId, origin: NodeId) {
        self.funcs.push(FuncCtx {
            id,
            name,
            slots: Vec::new(),
            blocks: Vec::new(),
            ret,
            ret_known: true,
            loop_depth: 0,
            lambdas: 0,
            origin,
        });
    }
}

// ---- Types -----------------------------------------------------------------

impl Lowerer<'_> {
    fn resolve_type(&mut self, node: NodeId) -> TypeId {
        match self.tree.kind(node) {
            NodeKind::NameType => self.resolve_type_name(node),
            NodeKind::UnitType => self.t_unit,
            NodeKind::FnType => {
                let mut params = Vec::new();
                let mut ret = self.t_unit;
                for child in cst::nodes(self.tree, node) {
                    if self.tree.kind(child) == NodeKind::RetType {
                        ret = match cst::type_child(self.tree, child) {
                            Some(ty) => self.resolve_type(ty),
                            None => self.t_error,
                        };
                    } else if cst::is_type(self.tree.kind(child)) {
                        params.push(self.resolve_type(child));
                    }
                }
                self.program.types.intern(TypeDef::Fn { params, ret })
            }
            _ => self.t_error,
        }
    }

    fn resolve_type_name(&mut self, node: NodeId) -> TypeId {
        let Some((name, span)) = cst::name(self.tree, self.source, node) else {
            return self.t_error;
        };
        match self.resolve(&name) {
            Some((_, Binding::Type(ty))) => ty,
            Some(_) => {
                self.error_at(DiagnosticKind::NotAType(name), span);
                self.t_error
            }
            None => {
                self.error_at(DiagnosticKind::UnknownType(name), span);
                self.t_error
            }
        }
    }
}
