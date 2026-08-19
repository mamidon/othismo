//! The executor: core IR in, a value out.
//!
//! There is no tree here and no names. Elaboration resolved every name to a
//! slot, checked every type, ordered every operand, and flattened every nested
//! expression, so what is left to do at run time is arithmetic, control flow,
//! and calls. That is the whole argument for goal §2.2's shared front end,
//! visible as a file that is mostly a `match` over eight statements.
//!
//! Three properties of the IR shape this walk:
//!
//! * **Every operand is atomic** (invariant 2), so [`Machine::operand`] never
//!   recurses and §15's "operands evaluate left to right" is nothing but the
//!   order of a statement list.
//! * **Blocks nest and nothing jumps** (invariant 3), so `break` and `continue`
//!   are a [`Flow`] returned up to the nearest [`Stmt::While`] rather than an
//!   edge to reconstruct.
//! * **Types live in slots** (invariant 1), so a value's width comes with it
//!   and §2's traps have something to check against.
//!
//! # What this stage runs
//!
//! Everything but §6's aggregates: constants, slots, arithmetic, comparison,
//! `if`, `while`, `break`, `continue`, `return`, calls direct and indirect,
//! and §5's closures and the cells that back a captured binding. Structs,
//! field access, and `as` are IR nodes this executor does not reach yet and
//! reports as [`TrapKind::Unsupported`] — which is a different thing to hear
//! than a crash, and each is scheduled to go away.

use ir::consts::{Const, ConstId};
use std::cell::RefCell;
use std::rc::Rc;

use ir::program::{BlockId, CstId, Func, FuncId, Operand, Place, Program, Rvalue, Stmt};
use ir::types::TypeDef;

use crate::error::{Trap, TrapKind};
use crate::ops;
use crate::value::{Closure, IntTy, Value};

/// How deep a call chain may go.
///
/// §5: there is no tail-call guarantee, so deep recursion exhausts the stack
/// and traps. A real stack overflow aborts the process without a message, which
/// is the one outcome worth ruling out — so the trap is raised at a depth the
/// host stack still has plenty of room for. The number is a budget, not a
/// language rule; §15 owns resource limits.
const RECURSION_LIMIT: usize = 256;

/// How a statement list ends.
///
/// §4's `break` and `continue` are unlabelled and apply to the innermost
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
    /// The constant pool, decoded once. §1's constants are frozen by the time
    /// they get here, so this is a table rather than an evaluation.
    consts: Vec<Value>,
    /// How many calls are open, against [`RECURSION_LIMIT`].
    depth: usize,
}

impl<'a> Machine<'a> {
    pub(crate) fn new(program: &'a Program) -> Machine<'a> {
        let consts = (0..program.consts.len())
            .map(|index| constant(program, ConstId(index as u32)))
            .collect();
        Machine {
            program,
            consts,
            depth: 0,
        }
    }

    /// Runs the file.
    ///
    /// §3 makes a file a block, and elaboration turns that block into an
    /// ordinary function — so "a bare expression is a valid program" (goal
    /// §2.1) needs nothing here beyond calling it.
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
    /// Every remaining slot is written before it is read — §3 requires every
    /// binding to have an initializer, and A-normal form assigns a temporary at
    /// the point it creates one — so what they start as is unobservable.
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
        debug_assert_eq!(func.params as usize, args.len(), "arity is checked (§5)");
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
            // §4's jumps cannot cross a call, so the other three are shapes the
            // IR does not have.
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
                let value = self.rvalue(rvalue, at, slots)?;
                slots[dst.index()] = value;
                Ok(Flow::Normal)
            }
            // §3's expression statement: evaluate, discard. Nothing marks the
            // discard as deliberate, because §3 doesn't.
            Stmt::Drop(rvalue) => {
                self.rvalue(rvalue, at, slots)?;
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
                    // §5: a function with no `-> T` returns unit — a real value
                    // with one inhabitant, not an absence.
                    None => Value::Unit,
                };
                Ok(Flow::Return(value))
            }
            Stmt::Store { place, value } => match place {
                // §5: writing through a cell is what makes one holder's
                // assignment visible to every other. An ordinary `Store`, not
                // an instruction of its own.
                Place::Cell(slot) => {
                    let value = self.operand(*value, slots);
                    match &slots[slot.index()] {
                        Value::Cell(cell) => *cell.borrow_mut() = value,
                        other => unreachable!("a cell slot holds `{}`", other.type_name()),
                    }
                    Ok(Flow::Normal)
                }
                Place::Field { .. } => {
                    Err(Trap::new(TrapKind::Unsupported("assigning to a field"), at))
                }
            },
        }
    }

    /// Run `header`, test `cond`, run `body`, repeat (§4).
    ///
    /// The header is inside the loop because a condition is re-evaluated every
    /// iteration and A-normal form has nowhere else to put its computation.
    /// `continue` therefore re-enters the header rather than skipping to the
    /// test, which is what makes it the same loop wasm's `loop` + `br_if` runs.
    ///
    /// A `break` or `continue` written *in* the condition — which needs a block
    /// expression, so `while { break; true } { … }` — belongs to this loop, on
    /// §4's reading that the innermost enclosing loop of a condition is the
    /// loop it conditions. Elaboration counts the header inside the loop for
    /// the same reason, so the two agree about which loop such a jump leaves.
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
                // §4: unlabelled, and applying to the innermost enclosing loop
                // — which is this one, so it travels no further.
                Flow::Break => return Ok(Flow::Normal),
                flow @ Flow::Return(_) => return Ok(flow),
            }
        }
    }

    fn rvalue(&mut self, rvalue: &Rvalue, at: CstId, slots: &mut [Value]) -> Result<Value, Trap> {
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
                // §2 and §5: arguments evaluate left to right, before the call —
                // which by now has already happened, since each is a slot or a
                // constant. This is the copy into the callee's frame.
                let args = self.operands(args, slots);
                self.call(*id, args, &[], at)
            }
            // §5: a call through a function *value*. On wasm this is
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
            // §5: every function value is a closure, and a plain `fn` referred
            // to by name is one with an empty environment.
            Rvalue::MakeClosure { func, captures } => {
                let captures = self.operands(captures, slots);
                Ok(Value::Closure(Rc::new(Closure {
                    func: *func,
                    captures,
                    name: self.name_of(*func),
                })))
            }
            Rvalue::MakeCell(operand) => {
                let value = self.operand(*operand, slots);
                Ok(Value::Cell(Rc::new(RefCell::new(value))))
            }
            Rvalue::CellGet(operand) => match self.operand(*operand, slots) {
                Value::Cell(cell) => Ok(cell.borrow().clone()),
                other => unreachable!("a cell read holds `{}`", other.type_name()),
            },
            Rvalue::Cast(_) => unsupported("the `as` operator", at),
            Rvalue::MakeStruct(_) => unsupported("a struct literal", at),
            Rvalue::Field { .. } => unsupported("field access", at),
        }
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

    /// §2: there is no truthiness. A condition is a `bool` by the time it gets
    /// here, because elaboration accepts nothing else.
    fn condition(&self, operand: Operand, slots: &[Value]) -> bool {
        match self.operand(operand, slots) {
            Value::Bool(value) => value,
            other => unreachable!("a condition is `bool`, and this is `{}`", other.type_name()),
        }
    }
}

fn unsupported(what: &'static str, at: CstId) -> Result<Value, Trap> {
    Err(Trap::new(TrapKind::Unsupported(what), at))
}

/// One entry of the constant pool as a value.
///
/// §1's integers are stored as bits and read back as their mathematical value,
/// sign-extended for a signed type — the same decode the dump does, and the
/// reason [`Value`] carries an [`IntTy`] rather than a width in bits.
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
        // §14's, and lowering builds none yet: a struct constant is what a
        // comptime evaluation freezes, and `comptime` has no token.
        Const::Struct { .. } => unreachable!("lowering produces no struct constant"),
    }
}
