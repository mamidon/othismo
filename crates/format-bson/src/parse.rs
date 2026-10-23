//! The BSON format's schema language, the body of a `schemas
//! application/bson { ... }` block: `import`s from other `.oth` files'
//! blocks, and `message` declarations.

use crate::Messages;
use crate::messages::{Field, FieldType, MessageType};
use anyhow::{Result, bail};
use std::collections::BTreeMap;
use syntax::image_definition::{Parser, Pos, Source, Tok};

/// The field types a field can be written with, besides a type's name.
const FIELD_TYPES: &str = "string, bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, bytes, time, uuid, a message type, a list of one of those, `[T]`, or a map from strings to one, `{T}`";

pub fn messages(src: &str, import: &mut dyn FnMut(&str) -> Result<Messages>) -> Result<Messages> {
    let mut p = Parser::new(src, true)?;
    let mut out = Messages::default();
    // Where each imported type came from: those named, and those that came
    // with them because their fields name them.
    let mut named: BTreeMap<String, String> = BTreeMap::new();
    let mut brought: BTreeMap<String, String> = BTreeMap::new();
    // Every type a field names, to look up once the whole block is read.
    let mut refs: Vec<(String, Pos)> = vec![];
    loop {
        let doc = p.docs();
        let pos = p.pos();
        match p.peek().clone() {
            Tok::Eof => {
                p.no_docs(&doc, pos)?;
                break;
            }
            Tok::Ident(kw) if kw == "import" => {
                p.no_docs(&doc, pos)?;
                let imp = p.import()?;
                let Source::Path(path) = &imp.from else { bail!("{pos}: message types are imported from other `.oth` files' `{}` schemas", crate::BSON) };
                let theirs = import(path).map_err(|e| anyhow::anyhow!("{pos}: importing from {path:?}: {e:#}"))?;
                for n in &imp.names {
                    let name = &n.name.name;
                    if n.local.name != *name {
                        bail!("{}: a message type is named on the wire, so `{name}` can't be renamed", n.local.pos);
                    }
                    if !theirs.types.contains_key(name) {
                        bail!("{}: {path:?} has no message type `{name}`", n.name.pos);
                    }
                    if named.insert(name.clone(), path.clone()).is_some() || out.declared.contains(name) {
                        bail!("{}: `{name}` is already imported", n.name.pos);
                    }
                    for dep in theirs.closure(name) {
                        let ty = &theirs.types[&dep];
                        match out.types.get(&dep) {
                            Some(mine) if mine.fields != ty.fields => {
                                bail!("{}: `{name}` needs {path:?}'s `{dep}`, which is not the `{dep}` already here", n.name.pos)
                            }
                            Some(_) => {}
                            None => {
                                out.types.insert(dep.clone(), ty.clone());
                            }
                        }
                        if dep != *name {
                            brought.entry(dep).or_insert_with(|| path.clone());
                        }
                    }
                }
            }
            Tok::Ident(kw) if kw == "message" => {
                p.next();
                let name = p.ident("message name", false)?;
                if let Some(prev) = named.get(&name.name) {
                    bail!("{}: `{}` is imported from {prev:?}, and declared here too", name.pos, name.name);
                }
                if let Some(prev) = brought.get(&name.name) {
                    bail!("{}: `{}` comes from {prev:?} with a type imported from there, and is declared here too", name.pos, name.name);
                }
                if out.types.contains_key(&name.name) {
                    bail!("{}: message `{}` is declared twice", name.pos, name.name);
                }
                p.expect(Tok::LBrace)?;
                let mut ty = MessageType { name: name.name.clone(), fields: vec![], doc, field_docs: vec![] };
                loop {
                    let field_doc = p.docs();
                    if *p.peek() == Tok::RBrace {
                        let pos = p.pos();
                        p.no_docs(&field_doc, pos)?;
                        p.next();
                        break;
                    }
                    let fname = p.ident("field name", false)?;
                    if ty.field(&fname.name).is_some() {
                        bail!("{}: `{}` has two fields called `{}`", fname.pos, name.name, fname.name);
                    }
                    let optional = *p.peek() == Tok::Question;
                    if optional {
                        p.next();
                    }
                    p.expect(Tok::Colon)?;
                    let fty = field_type(&mut p, &mut refs)?;
                    ty.fields.push(Field { optional, ..Field::new(fname.name, fty) });
                    ty.field_docs.push(field_doc);
                    if *p.peek() == Tok::Comma {
                        p.next();
                    }
                }
                out.types.insert(name.name.clone(), ty);
                out.declared.push(name.name);
            }
            other => bail!("{pos}: expected `import` or `message`, found {other}"),
        }
    }
    for (name, pos) in refs {
        if !out.types.contains_key(&name) {
            bail!("{pos}: `{name}` is not a field type, nor a message type declared or imported here; fields are {FIELD_TYPES}");
        }
    }
    Ok(out)
}

/// A field's type: `[T]`, `{T}`, or a name. A name that is not a field type's is a message type's,
/// which is noted in `refs` to look up later.
fn field_type(p: &mut Parser, refs: &mut Vec<(String, Pos)>) -> Result<FieldType> {
    if *p.peek() == Tok::LBracket {
        p.next();
        let t = field_type(p, refs)?;
        p.expect(Tok::RBracket)?;
        return Ok(FieldType::List(Box::new(t)));
    }
    if *p.peek() == Tok::LBrace {
        p.next();
        let t = field_type(p, refs)?;
        p.expect(Tok::RBrace)?;
        return Ok(FieldType::Map(Box::new(t)));
    }
    let name = p.ident("a field type", false)?;
    Ok(match FieldType::from_name(&name.name) {
        Some(t) => t,
        None => {
            refs.push((name.name.clone(), name.pos));
            FieldType::Named(name.name)
        }
    })
}
