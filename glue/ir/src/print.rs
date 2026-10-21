//! The debug rendering: s-expressions.
//!
//! Every node is a form — `(kind desc)` for a leaf, or `(kind desc` with one
//! child per line and the closing paren trailing the last of them. Four
//! conventions keep it from drowning in parentheses:
//!
//! * **A slot in operand position is its bare name.** Anything parenthesized
//!   is a form, so nothing is ambiguous. A temporary has no source name and is
//!   written `t3`; a name that two slots share — which shadowing makes
//!   ordinary (§statements) — is disambiguated with its slot index.
//! * **A constant is `(const …)`**, carrying its suffix so its type shows.
//! * **[`Rvalue::Use`] is elided.** A bare operand where an rvalue is expected
//!   is a use.
//! * **The integer after `slot`, `block`, `header`, `body`, `then`, and `else`
//!   is the arena index**, so a dump can be read against the data.
//! * **A slot that permits in-place mutation says `mut`** after its kind
//!   (§statements). A slot that does not says nothing, since that is the
//!   default.
//!
//! Blocks nest here rather than being listed flat and referenced by id. That is
//! a property of the *rendering*, not of the data — [`Func`] holds a flat
//! `Vec<Block>` and the statements hold [`BlockId`]s. The dump can nest them
//! because control flow is a tree, which is the same fact that makes the wasm
//! back end a translation rather than a reconstruction.

use std::collections::HashMap;

use crate::consts::Const;
use crate::program::{Block, BlockId, Func, GlobalId, Operand, Place, Program, Rvalue, Slot, Stmt};
use crate::types::{FieldIdx, TypeDef, TypeId};

/// The whole program: its struct types, then its functions.
pub fn dump(program: &Program) -> String {
    let mut out = String::new();

    for id in 0..program.types.len() {
        let ty = TypeId(id as u32);
        if let Some(def) = program.types.struct_def(ty) {
            let mut fields = Vec::new();
            for field in &def.fields {
                fields.push(Sexp::leaf(format!(
                    "(field {} {})",
                    program.text(field.name),
                    program.type_name(field.ty)
                )));
            }
            let name = def
                .name
                .map(|name| program.text(name).to_string())
                .unwrap_or_else(|| format!("struct{id}"));
            out.push_str(&Sexp::list(format!("struct {name}"), fields).render(0));
            out.push_str("\n\n");
        }
    }

    // §statements' top-level bindings. Listed ahead of the functions because
    // every one of them may read one.
    for (index, global) in program.globals.iter().enumerate() {
        out.push_str(&format!(
            "(global {} {}{})\n",
            global_text(program, GlobalId(index as u32)),
            program.type_name(global.ty),
            if global.mutable { " mut" } else { "" },
        ));
    }
    if !program.globals.is_empty() {
        out.push('\n');
    }

    for (index, func) in program.funcs.iter().enumerate() {
        out.push_str(&func_sexp(program, func).render(0));
        if program.entry == Some(crate::program::FuncId(index as u32)) {
            out.push_str("   ; the file's top level (§statements)");
        }
        out.push_str("\n\n");
    }

    out.trim_end().to_string()
}

pub fn dump_func(program: &Program, func: &Func) -> String {
    func_sexp(program, func).render(0)
}

fn func_sexp(program: &Program, func: &Func) -> Sexp {
    let names = SlotNames::compute(func, program);
    let cx = Cx {
        program,
        func,
        names: &names,
    };

    let params: Vec<String> = func.slots[..func.params as usize]
        .iter()
        .map(|slot| program.type_name(slot.ty))
        .collect();
    let signature = if matches!(program.types.get(func.ret), TypeDef::Unit) {
        format!("({})", params.join(" "))
    } else {
        format!("({}) -> {}", params.join(" "), program.type_name(func.ret))
    };
    let name = func
        .name
        .map(|name| program.text(name).to_string())
        .unwrap_or_else(|| "?".to_string());

    let mut children = Vec::new();
    let width = func
        .slots
        .iter()
        .enumerate()
        .map(|(index, _)| names.of(Slot(index as u32)).len())
        .max()
        .unwrap_or(0);
    for (index, slot) in func.slots.iter().enumerate() {
        let display = names.of(Slot(index as u32));
        // §statements' `mut` trails the kind, so a slot that permits in-place
        // mutation reads as `param mut` and one that does not is unchanged.
        let mutable = if slot.mutable { " mut" } else { "" };
        children.push(Sexp::leaf(format!(
            "(slot {index} {display:width$} {} {}{mutable})",
            program.type_name(slot.ty),
            slot.kind.name(),
        )));
    }
    children.push(block_sexp(&cx, "block", func.body()));

    Sexp::list(format!("func {name} {signature}"), children)
}

struct Cx<'a> {
    program: &'a Program,
    func: &'a Func,
    names: &'a SlotNames,
}

/// A display name per slot: the source name where it is unique in the
/// function, the name plus its index where §statements' shadowing made it
/// ambiguous, and `tN` for a temporary.
struct SlotNames {
    names: Vec<String>,
}

impl SlotNames {
    fn compute(func: &Func, program: &Program) -> SlotNames {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for slot in &func.slots {
            if let Some(name) = slot.name {
                *counts.entry(program.text(name)).or_default() += 1;
            }
        }
        let names = func
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| match slot.name {
                Some(name) if counts[program.text(name)] == 1 => program.text(name).to_string(),
                Some(name) => format!("{}.{index}", program.text(name)),
                None => format!("t{index}"),
            })
            .collect();
        SlotNames { names }
    }

    fn of(&self, slot: Slot) -> &str {
        &self.names[slot.index()]
    }
}

fn block_sexp(cx: &Cx, head: &str, id: BlockId) -> Sexp {
    let block: &Block = cx.func.block(id);
    let children = block.stmts.iter().map(|stmt| stmt_sexp(cx, stmt)).collect();
    Sexp::list(format!("{head} {}", id.index()), children)
}

fn stmt_sexp(cx: &Cx, stmt: &Stmt) -> Sexp {
    match stmt {
        Stmt::Assign { dst, rvalue } => Sexp::leaf(format!(
            "(assign {} {})",
            cx.names.of(*dst),
            rvalue_text(cx, rvalue)
        )),
        Stmt::Store { place, value } => Sexp::leaf(format!(
            "(store {} {})",
            place_text(cx, place),
            operand_text(cx, *value)
        )),
        Stmt::If { cond, then_, else_ } => Sexp::list(
            format!("if {}", operand_text(cx, *cond)),
            vec![
                block_sexp(cx, "then", *then_),
                block_sexp(cx, "else", *else_),
            ],
        ),
        Stmt::While { header, cond, body } => Sexp::list(
            "while".to_string(),
            vec![
                block_sexp(cx, "header", *header),
                Sexp::leaf(format!("(cond {})", operand_text(cx, *cond))),
                block_sexp(cx, "body", *body),
            ],
        ),
        Stmt::Break => Sexp::leaf("(break)".to_string()),
        Stmt::Continue => Sexp::leaf("(continue)".to_string()),
        Stmt::Return(None) => Sexp::leaf("(return)".to_string()),
        Stmt::Return(Some(operand)) => {
            Sexp::leaf(format!("(return {})", operand_text(cx, *operand)))
        }
        Stmt::Drop(rvalue) => Sexp::leaf(format!("(drop {})", rvalue_text(cx, rvalue))),
    }
}

/// An rvalue as a form, or — for [`Rvalue::Use`] — as a bare operand, since a
/// use is the absence of an operation.
fn rvalue_text(cx: &Cx, rvalue: &Rvalue) -> String {
    match rvalue {
        Rvalue::Use(operand) => operand_text(cx, *operand),
        Rvalue::Unary(op, operand) => {
            format!("({} {})", op.name(), operand_text(cx, *operand))
        }
        Rvalue::Binary(op, left, right) => format!(
            "({} {} {})",
            op.name(),
            operand_text(cx, *left),
            operand_text(cx, *right)
        ),
        Rvalue::Cast(operand) => format!("(cast {})", operand_text(cx, *operand)),
        Rvalue::Call { func, args } => {
            let name = cx
                .program
                .func(*func)
                .name
                .map(|name| cx.program.text(name).to_string())
                .unwrap_or_else(|| format!("fn{}", func.index()));
            format!("(call {name}{})", arg_text(cx, args))
        }
        Rvalue::CallIndirect { callee, args } => format!(
            "(call-indirect {}{})",
            operand_text(cx, *callee),
            arg_text(cx, args)
        ),
        Rvalue::MakeStruct(fields) => format!("(struct{})", arg_text(cx, fields)),
        Rvalue::Field { base, field } => format!(
            "(field {} {})",
            operand_text(cx, *base),
            field_text(cx, *base, *field)
        ),
        Rvalue::GlobalGet(id) => format!("(globalget {})", global_text(cx.program, *id)),
        Rvalue::MakeCell(operand) => format!("(makecell {})", operand_text(cx, *operand)),
        Rvalue::CellGet(operand) => format!("(cellget {})", operand_text(cx, *operand)),
        Rvalue::MakeClosure { func, captures } => {
            let name = cx
                .program
                .func(*func)
                .name
                .map(|name| cx.program.text(name).to_string())
                .unwrap_or_else(|| format!("fn{}", func.index()));
            if captures.is_empty() {
                format!("(closure {name})")
            } else {
                format!("(closure {name} (captures{}))", arg_text(cx, captures))
            }
        }
    }
}

fn arg_text(cx: &Cx, args: &[Operand]) -> String {
    args.iter()
        .map(|arg| format!(" {}", operand_text(cx, *arg)))
        .collect()
}

fn place_text(cx: &Cx, place: &Place) -> String {
    match place {
        Place::Cell(slot) => format!("(cell {})", cx.names.of(*slot)),
        Place::Global(id) => global_text(cx.program, *id),
        Place::Field { base, field } => format!(
            "(field {} {})",
            cx.names.of(*base),
            field_text(cx, Operand::Slot(*base), *field)
        ),
    }
}

/// A field by name where the base's type still knows it, and by index
/// otherwise.
fn field_text(cx: &Cx, base: Operand, field: FieldIdx) -> String {
    let ty = match base {
        Operand::Slot(slot) => cx.func.slot_ty(slot),
        Operand::Const(id) => match cx.program.consts.get(id) {
            Const::Struct { ty, .. } => *ty,
            _ => return field.0.to_string(),
        },
    };
    match cx.program.types.struct_def(ty) {
        Some(def) => match def.fields.get(field.0 as usize) {
            Some(def) => cx.program.text(def.name).to_string(),
            None => field.0.to_string(),
        },
        None => field.0.to_string(),
    }
}

/// A global reads as `@name`, so a dump never confuses one with a slot — and
/// gains its index where §statements' shadowing bound the same name twice,
/// which is the rule [`SlotNames`] follows for slots.
fn global_text(program: &Program, id: GlobalId) -> String {
    let name = program.text(program.global(id).name);
    let shadowed = program
        .globals
        .iter()
        .filter(|other| program.text(other.name) == name)
        .count()
        > 1;
    if shadowed {
        format!("@{name}.{}", id.index())
    } else {
        format!("@{name}")
    }
}

fn operand_text(cx: &Cx, operand: Operand) -> String {
    match operand {
        Operand::Slot(slot) => cx.names.of(slot).to_string(),
        Operand::Const(id) => format!("(const {})", const_text(cx.program, id)),
    }
}

fn const_text(program: &Program, id: crate::consts::ConstId) -> String {
    match program.consts.get(id) {
        Const::Unit => "()".to_string(),
        Const::Bool(value) => value.to_string(),
        Const::Char(value) => format!("{value:?}"),
        Const::Str(value) => format!("{value:?}"),
        Const::Int { ty, bits } => {
            let suffix = program.type_name(*ty);
            match program.types.get(*ty) {
                // Sign-extend from the type's width, so a negative constant
                // reads as one rather than as a very large unsigned.
                TypeDef::Int {
                    signed: true,
                    bits: width,
                } => {
                    let shift = 64 - *width as u32;
                    format!("{}{suffix}", ((*bits as i64) << shift) >> shift)
                }
                _ => format!("{bits}{suffix}"),
            }
        }
        Const::Float { ty, bits } => {
            let value = f64::from_bits(*bits);
            format!("{value:?}{}", program.type_name(*ty))
        }
        Const::Struct { ty, fields } => {
            let fields: String = fields
                .iter()
                .map(|field| format!(" {}", const_text(program, *field)))
                .collect();
            format!("(struct {}{fields})", program.type_name(*ty))
        }
    }
}

// ---- The renderer ----------------------------------------------------------

enum Sexp {
    Leaf(String),
    List(String, Vec<Sexp>),
}

impl Sexp {
    fn leaf(text: String) -> Sexp {
        Sexp::Leaf(text)
    }

    fn list(head: String, children: Vec<Sexp>) -> Sexp {
        Sexp::List(head, children)
    }

    fn render(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self {
            Sexp::Leaf(text) => format!("{pad}{text}"),
            Sexp::List(head, children) if children.is_empty() => format!("{pad}({head})"),
            Sexp::List(head, children) => {
                let mut out = format!("{pad}({head}");
                for child in children {
                    out.push('\n');
                    out.push_str(&child.render(indent + 2));
                }
                out.push(')');
                out
            }
        }
    }
}
