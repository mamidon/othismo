//! The executor: core IR in, a value out.
//!
//! There is no tree here and no names. Elaboration resolved every name to a
//! slot, checked every type, ordered every operand, and flattened every nested
//! expression, so what is left to do at run time is arithmetic, control flow,
//! and calls. That is the whole argument for goal §both-modes' shared front
//! end, visible as a file that is mostly a `match` over eight statements.
//!
//! Three properties of the IR shape this walk:
//!
//! * **Every operand is atomic** (invariant 2), so [`Machine::operand`] never
//!   recurses and §semantics' "operands evaluate left to right" is nothing
//!   but the order of a statement list.
//! * **Blocks nest and nothing jumps** (invariant 3), so `break` and `continue`
//!   are a [`Flow`] returned up to the nearest [`Stmt::While`] rather than an
//!   edge to reconstruct.
//! * **Types live in slots** (invariant 1), so a value's width comes with it
//!   and §expressions' traps have something to check against.
//!
//! # What this stage runs
//!
//! Every node core IR has today. What is missing from the language is missing
//! from the IR first, and `ir`'s own documentation is where that list lives:
//! `CallHost` (§modules), `Index` and the collections that would give it a
//! meaning (§types, §generics), `match` (§unions), and generics (§generics,
//! §comptime).
//!
//! # Where an instruction's type comes from
//!
//! Invariant 1 says types live in slots, so the two instructions that need one
//! take it from their *destination*: [`Rvalue::Cast`] converts to the
//! destination slot's type and [`Rvalue::MakeStruct`] builds an instance of it.
//! That is why [`Machine::rvalue`] is handed a type, and why there is exactly
//! one place a type can be wrong.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ir::consts::{Const, ConstId};
use ir::program::{BlockId, CstId, Func, FuncId, Operand, Place, Program, Rvalue, Stmt};
use ir::types::{TypeDef, TypeId};

use crate::ops;
use crate::trap::{Trap, TrapKind};
use crate::value::{Closure, IntTy, StructShape, StructVal, Value};

/// How deep a call chain may go.
///
/// §functions: there is no tail-call guarantee, so deep recursion exhausts the
/// stack and traps. A real stack overflow aborts the process without a
/// message, which is the one outcome worth ruling out — so the trap is raised
/// at a depth the host stack still has plenty of room for. The number is a
/// budget, not a language rule; §semantics owns resource limits.
const RECURSION_LIMIT: usize = 256;

/// How a statement list ends.
///
/// §control's `break` and `continue` are unlabelled and apply to the innermost
/// enclosing loop, which — since blocks nest — is whichever [`Stmt::While`] is
/// nearest up the call chain of [`Machine::block`]. `return` travels the same
/// way and is caught one level further out, at the call.
enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
}

pub(crate) struct Machine<'a> {
    program: &'a Program,
    /// The constant pool, decoded once. §lexical's constants are frozen by the
    /// time they get here, so this is a table rather than an evaluation.
    consts: Vec<Value>,
    /// What each struct type is called, and what its fields are called — for
    /// echoing an instance, and for nothing else. Field access is positional.
    shapes: HashMap<TypeId, Rc<StructShape>>,
    /// §statements' top-level bindings. One store per global, outside every
    /// frame — which is what a `fn` reads when it names one, and why reading
    /// it needs no environment (§functions).
    ///
    /// They start as unit and are unobservable until written: a global's `let`
    /// is a statement of the entry function, and elaboration refuses any
    /// program that could read one before that statement runs.
    globals: Vec<Value>,
    /// How many calls are open, against [`RECURSION_LIMIT`].
    depth: usize,
}

impl<'a> Machine<'a> {
    pub(crate) fn new(program: &'a Program) -> Machine<'a> {
        let consts = (0..program.consts.len())
            .map(|index| constant(program, ConstId(index as u32)))
            .collect();
        let shapes = (0..program.types.len())
            .map(|index| TypeId(index as u32))
            .filter_map(|ty| Some((ty, shape_of(program, ty)?)))
            .collect();
        Machine {
            program,
            consts,
            shapes,
            globals: vec![Value::Unit; program.globals.len()],
            depth: 0,
        }
    }

    /// Runs the file.
    ///
    /// §statements makes a file a block, and elaboration turns that block into
    /// an ordinary function — so "a bare expression is a valid program" (goal
    /// §one-language) needs nothing here beyond calling it.
    pub(crate) fn run(mut self) -> Result<Value, Trap> {
        let Some(entry) = self.program.entry else {
            return Ok(Value::Unit);
        };
        let at = self.program.func(entry).origin;
        self.call(entry, Vec::new(), &[], at)
    }

    /// Runs a function against a frame of its own.
    ///
    /// The frame is `slots` long and laid out `[params][captures][locals]`, so
    /// entering a function is two copies into the front of it and no lookup —
    /// that layout is the calling convention, and this is the whole of it.
    /// Every remaining slot is written before it is read — §statements
    /// requires every binding to have an initializer, and A-normal form
    /// assigns a temporary at the point it creates one — so what they start as
    /// is unobservable.
    fn call(
        &mut self,
        id: FuncId,
        args: Vec<Value>,
        captures: &[Value],
        at: CstId,
    ) -> Result<Value, Trap> {
        if self.depth >= RECURSION_LIMIT {
            return Err(Trap::new(TrapKind::RecursionLimit, at));
        }
        let func = self.program.func(id);
        debug_assert_eq!(
            func.params as usize,
            args.len(),
            "arity is checked (§functions)"
        );
        debug_assert_eq!(
            func.captures as usize,
            captures.len(),
            "captures are positional"
        );

        let mut slots = vec![Value::Unit; func.slots.len()];
        for (slot, value) in slots.iter_mut().zip(args) {
            *slot = value;
        }
        for (slot, value) in slots[func.params as usize..].iter_mut().zip(captures) {
            *slot = value.clone();
        }

        self.depth += 1;
        let flow = self.block(func, func.body(), &mut slots);
        self.depth -= 1;

        match flow? {
            Flow::Return(value) => Ok(value),
            // Elaboration ends every body with a `return` (`emit_result`), and
            // §control's jumps cannot cross a call, so the other three are
            // shapes the IR does not have.
            _ => unreachable!("a function body ends in a return"),
        }
    }

    fn block(&mut self, func: &'a Func, id: BlockId, slots: &mut [Value]) -> Result<Flow, Trap> {
        let block = func.block(id);
        for (stmt, at) in block.stmts.iter().zip(&block.spans) {
            match self.stmt(func, stmt, *at, slots)? {
                Flow::Normal => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }

    fn stmt(
        &mut self,
        func: &'a Func,
        stmt: &Stmt,
        at: CstId,
        slots: &mut [Value],
    ) -> Result<Flow, Trap> {
        match stmt {
            Stmt::Assign { dst, rvalue } => {
                // Invariant 1: the destination's type is the instruction's,
                // for the two rvalues that take one from it.
                let value = self.rvalue(rvalue, Some(func.slot_ty(*dst)), at, slots)?;
                slots[dst.index()] = value;
                Ok(Flow::Normal)
            }
            // §statements' expression statement: evaluate, discard. Nothing
            // marks the discard as deliberate, because §statements doesn't.
            Stmt::Drop(rvalue) => {
                // No destination, and so no type — sound because elaboration
                // discards nothing but a call.
                self.rvalue(rvalue, None, at, slots)?;
                Ok(Flow::Normal)
            }
            Stmt::If { cond, then_, else_ } => {
                let taken = if self.condition(*cond, slots) {
                    *then_
                } else {
                    *else_
                };
                self.block(func, taken, slots)
            }
            Stmt::While { header, cond, body } => self.while_(func, *header, *cond, *body, slots),
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Return(value) => {
                let value = match value {
                    Some(operand) => self.operand(*operand, slots),
                    // §functions: a function with no `-> T` returns unit — a
                    // real value with one inhabitant, not an absence.
                    None => Value::Unit,
                };
                Ok(Flow::Return(value))
            }
            Stmt::Store { place, value } => match place {
                // §functions: writing through a cell is what makes one
                // holder's assignment visible to every other. An ordinary
                // `Store`, not an instruction of its own.
                Place::Cell(slot) => {
                    let value = self.operand(*value, slots);
                    match &slots[slot.index()] {
                        Value::Cell(cell) => *cell.borrow_mut() = value,
                        other => unreachable!("a cell slot holds `{}`", other.type_name()),
                    }
                    Ok(Flow::Normal)
                }
                // §statements: initializing a top-level binding, and
                // assigning one, are the same store.
                Place::Global(id) => {
                    let value = self.operand(*value, slots);
                    self.globals[id.index()] = value;
                    Ok(Flow::Normal)
                }
                // §types: assignment through a reference, which every other
                // holder of that instance observes. `mut` on the binding is
                // what permitted it, and elaboration checked that.
                Place::Field { base, field } => {
                    let value = self.operand(*value, slots);
                    match &slots[base.index()] {
                        Value::Struct(instance) => {
                            instance.fields.borrow_mut()[field.0 as usize] = value
                        }
                        other => unreachable!(
                            "a field is assigned on a struct, not `{}`",
                            other.type_name()
                        ),
                    }
                    Ok(Flow::Normal)
                }
            },
        }
    }

    /// Run `header`, test `cond`, run `body`, repeat (§control).
    ///
    /// The header is inside the loop because a condition is re-evaluated every
    /// iteration and A-normal form has nowhere else to put its computation.
    /// `continue` therefore re-enters the header rather than skipping to the
    /// test, which is what makes it the same loop wasm's `loop` + `br_if` runs.
    ///
    /// A `break` or `continue` written *in* the condition — which needs a
    /// block expression, so `while { break; true } { … }` — belongs to this
    /// loop, on §control's reading that the innermost enclosing loop of a
    /// condition is the loop it conditions. Elaboration counts the header
    /// inside the loop for the same reason, so the two agree about which loop
    /// such a jump leaves.
    fn while_(
        &mut self,
        func: &'a Func,
        header: BlockId,
        cond: Operand,
        body: BlockId,
        slots: &mut [Value],
    ) -> Result<Flow, Trap> {
        loop {
            match self.block(func, header, slots)? {
                Flow::Normal | Flow::Continue => {}
                Flow::Break => return Ok(Flow::Normal),
                flow @ Flow::Return(_) => return Ok(flow),
            }
            if !self.condition(cond, slots) {
                return Ok(Flow::Normal);
            }
            match self.block(func, body, slots)? {
                Flow::Normal | Flow::Continue => {}
                // §control: unlabelled, and applying to the innermost
                // enclosing loop — which is this one, so it travels no
                // further.
                Flow::Break => return Ok(Flow::Normal),
                flow @ Flow::Return(_) => return Ok(flow),
            }
        }
    }

    fn rvalue(
        &mut self,
        rvalue: &Rvalue,
        ty: Option<TypeId>,
        at: CstId,
        slots: &mut [Value],
    ) -> Result<Value, Trap> {
        match rvalue {
            Rvalue::Use(operand) => Ok(self.operand(*operand, slots)),
            Rvalue::Unary(op, operand) => {
                let operand = self.operand(*operand, slots);
                ops::unary(*op, operand).map_err(|kind| Trap::new(kind, at))
            }
            Rvalue::Binary(op, left, right) => {
                let left = self.operand(*left, slots);
                let right = self.operand(*right, slots);
                ops::binary(*op, left, right).map_err(|kind| Trap::new(kind, at))
            }
            Rvalue::Call { func: id, args } => {
                // §expressions and §functions: arguments evaluate left to
                // right, before the call — which by now has already happened,
                // since each is a slot or a constant. This is the copy into
                // the callee's frame.
                let args = self.operands(args, slots);
                self.call(*id, args, &[], at)
            }
            // §functions: a call through a function *value*. On wasm this is
            // `call_indirect`; here it is the same call with an environment.
            Rvalue::CallIndirect { callee, args } => {
                let callee = match self.operand(*callee, slots) {
                    Value::Closure(closure) => closure,
                    other => unreachable!(
                        "a callee is a function, and this is `{}`",
                        other.type_name()
                    ),
                };
                let args = self.operands(args, slots);
                self.call(callee.func, args, &callee.captures, at)
            }
            // §functions: every function value is a closure, and a plain `fn`
            // referred to by name is one with an empty environment.
            Rvalue::MakeClosure { func, captures } => {
                let captures = self.operands(captures, slots);
                Ok(Value::Closure(Rc::new(Closure {
                    func: *func,
                    captures,
                    name: self.name_of(*func),
                })))
            }
            // §statements: one load from storage that outlives every frame.
            // `global.get` on wasm.
            Rvalue::GlobalGet(id) => Ok(self.globals[id.index()].clone()),
            Rvalue::MakeCell(operand) => {
                let value = self.operand(*operand, slots);
                Ok(Value::Cell(Rc::new(RefCell::new(value))))
            }
            Rvalue::CellGet(operand) => match self.operand(*operand, slots) {
                Value::Cell(cell) => Ok(cell.borrow().clone()),
                other => unreachable!("a cell read holds `{}`", other.type_name()),
            },
            // §expressions: explicit and trapping. The target is the
            // destination slot's type, which is invariant 1 in the one place
            // it is load-bearing.
            Rvalue::Cast(operand) => {
                let value = self.operand(*operand, slots);
                self.cast(value, self.expect_type(ty))
                    .map_err(|kind| Trap::new(kind, at))
            }
            // §types: fields in declaration order — §expressions'
            // left-to-right evaluation of the *written* order already happened
            // in the statements above this one.
            Rvalue::MakeStruct(fields) => {
                let fields = self.operands(fields, slots);
                Ok(Value::Struct(Rc::new(StructVal {
                    shape: self.shape(self.expect_type(ty)),
                    fields: RefCell::new(fields),
                })))
            }
            Rvalue::Field { base, field } => match self.operand(*base, slots) {
                Value::Struct(value) => Ok(value.fields.borrow()[field.0 as usize].clone()),
                other => {
                    unreachable!("a field is read from a struct, not `{}`", other.type_name())
                }
            },
        }
    }

    /// §expressions' conversion table, which lowering has already folded for
    /// constant operands (`cast_const`) and hands here for everything else.
    /// Truncation and rounding are defined behaviour; the conversion with no
    /// representable answer traps.
    fn cast(&self, value: Value, target: TypeId) -> Result<Value, TrapKind> {
        let out_of_range = |shown: String| TrapKind::CastOutOfRange {
            value: shown,
            ty: self.program.type_name(target),
        };
        match (value, self.program.types.get(target)) {
            // Integer to integer: exact, or a trap. There is no wrapping here
            // any more than there is in `+`.
            (Value::Int { value, .. }, TypeDef::Int { signed, bits }) => {
                let ty = IntTy::new(*signed, *bits);
                if ty.holds(value) {
                    Ok(Value::Int { value, ty })
                } else {
                    Err(out_of_range(value.to_string()))
                }
            }
            // Integer to float rounds, which is what `f64` does to an `i128`
            // it cannot hold exactly.
            (Value::Int { value, .. }, TypeDef::Float { bits }) => Ok(float(value as f64, *bits)),
            // Float to integer truncates toward zero, and traps at the edges —
            // NaN and the infinities have no representable answer at all.
            (Value::Float { value, .. }, TypeDef::Int { signed, bits }) => {
                let ty = IntTy::new(*signed, *bits);
                let truncated = value.trunc();
                if !truncated.is_finite()
                    || truncated < ty.min() as f64
                    || truncated > ty.max() as f64
                {
                    return Err(out_of_range(value.to_string()));
                }
                Ok(Value::Int {
                    value: truncated as i128,
                    ty,
                })
            }
            (Value::Float { value, .. }, TypeDef::Float { bits }) => Ok(float(value, *bits)),
            (value, _) => unreachable!(
                "elaboration allows no conversion from `{}` to `{}`",
                value.type_name(),
                self.program.type_name(target)
            ),
        }
    }

    fn shape(&self, ty: TypeId) -> Rc<StructShape> {
        match self.shapes.get(&ty) {
            Some(shape) => Rc::clone(shape),
            None => unreachable!("a struct is made at a struct type"),
        }
    }

    /// The destination slot's type, for the two instructions that take theirs
    /// from it (invariant 1).
    fn expect_type(&self, ty: Option<TypeId>) -> TypeId {
        ty.expect("this instruction takes its type from its destination slot")
    }

    fn operands(&self, operands: &[Operand], slots: &[Value]) -> Vec<Value> {
        operands
            .iter()
            .map(|operand| self.operand(*operand, slots))
            .collect()
    }

    /// What to call a function value when a person looks at one. Elaboration
    /// names every function, lambdas included — `counter.λ0` is one.
    fn name_of(&self, func: FuncId) -> Rc<str> {
        match self.program.func(func).name {
            Some(name) => Rc::from(self.program.text(name)),
            None => Rc::from("?"),
        }
    }

    fn operand(&self, operand: Operand, slots: &[Value]) -> Value {
        match operand {
            Operand::Slot(slot) => slots[slot.index()].clone(),
            Operand::Const(id) => self.consts[id.index()].clone(),
        }
    }

    /// §expressions: there is no truthiness. A condition is a `bool` by the
    /// time it gets here, because elaboration accepts nothing else.
    fn condition(&self, operand: Operand, slots: &[Value]) -> bool {
        match self.operand(operand, slots) {
            Value::Bool(value) => value,
            other => unreachable!("a condition is `bool`, and this is `{}`", other.type_name()),
        }
    }
}

/// A float at the width its type says, rounded through `f32` when that is the
/// width — the same rule [`crate::ops`] applies after every operation.
fn float(value: f64, bits: u8) -> Value {
    Value::Float {
        value: if bits == 32 {
            value as f32 as f64
        } else {
            value
        },
        bits,
    }
}

/// What to call a struct type and its fields, if it is one.
fn shape_of(program: &Program, ty: TypeId) -> Option<Rc<StructShape>> {
    let def = program.types.struct_def(ty)?;
    Some(Rc::new(StructShape {
        name: match def.name {
            Some(name) => Rc::from(program.text(name)),
            // §comptime's anonymous `struct { … }`, which lowering cannot
            // produce yet.
            None => Rc::from("struct"),
        },
        fields: def
            .fields
            .iter()
            .map(|field| Rc::from(program.text(field.name)))
            .collect(),
    }))
}

/// One entry of the constant pool as a value.
///
/// §lexical's integers are stored as bits and read back as their mathematical
/// value, sign-extended for a signed type — the same decode the dump does, and
/// the reason [`Value`] carries an [`IntTy`] rather than a width in bits.
fn constant(program: &Program, id: ConstId) -> Value {
    match program.consts.get(id) {
        Const::Unit => Value::Unit,
        Const::Bool(value) => Value::Bool(*value),
        Const::Int { ty, bits } => match program.types.get(*ty) {
            TypeDef::Int {
                signed,
                bits: width,
            } => {
                let ty = IntTy::new(*signed, *width);
                let value = if *signed {
                    // Sign-extend from the type's width.
                    let shift = 128 - *width as u32;
                    ((*bits as i128) << shift) >> shift
                } else {
                    *bits as i128 & ty.max()
                };
                Value::Int { value, ty }
            }
            _ => unreachable!("an integer constant has an integer type"),
        },
        Const::Float { ty, bits } => match program.types.get(*ty) {
            TypeDef::Float { bits: width } => Value::Float {
                value: f64::from_bits(*bits),
                bits: *width,
            },
            _ => unreachable!("a float constant has a float type"),
        },
        Const::Char(value) => Value::Char(*value),
        Const::Str(text) => Value::Str(text.clone()),
        // §comptime's, and lowering builds none yet: a struct constant is what
        // a comptime evaluation freezes, and nothing evaluates one yet.
        Const::Struct { .. } => unreachable!("lowering produces no struct constant"),
    }
}
