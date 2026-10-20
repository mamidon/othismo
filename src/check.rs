//! Resolves and validates a parsed image into a [`Program`] the runtime can run.

use crate::image::{self, Ident, Image, Value};
use crate::messages::{self, FieldType, MessageType};
use crate::wire::{self, Message};
use anyhow::{Context, Result, bail};
use bson::{Bson, Document};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const STDIN: &str = "Environment.Stdin";
pub const STDOUT: &str = "Environment.Stdout";

/// The message type `Environment.Stdin` emits. Its declaration must be
/// exactly `{ text: string }`.
pub const LINE: &str = "Line";

#[derive(Debug)]
pub struct Program {
    pub messages: BTreeMap<String, MessageType>,
    /// Instances in image order; [`Port::targets`] index into this.
    pub instances: Vec<Instance>,
}

#[derive(Debug)]
pub struct Instance {
    pub name: String,
    pub kind: Kind,
    /// Out ports in protocol declaration order (the order constructors see).
    pub ports: Vec<Port>,
    pub startup: Vec<Message>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Wasm { protocol: String, module: PathBuf },
    Stdin,
    Stdout,
}

#[derive(Debug)]
pub struct Port {
    pub name: String,
    pub types: Vec<String>,
    pub targets: Vec<usize>,
}

/// What a protocol accepts on input.
enum Accepts {
    Only(Vec<String>),
    Anything,
}

/// A protocol after resolution; built-ins and image-declared ones alike.
struct ProtocolDef {
    kind: Kind,
    accepts: Accepts,
    outputs: Vec<(String, Vec<String>)>,
}

fn err_at(id: &Ident, msg: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{}: {msg}", id.pos)
}

/// Reads, parses and checks the image at `path`.
pub fn load(path: &Path) -> Result<Program> {
    let src = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let img = image::parse(&src).with_context(|| format!("parsing {}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    check(&img, dir).with_context(|| format!("checking {}", path.display()))
}

/// Validates `img`; relative paths (imports, `scripts/`) resolve against `dir`.
pub fn check(img: &Image, dir: &Path) -> Result<Program> {
    let messages = resolve_imports(img, dir)?;
    let lookup_type = |id: &Ident| -> Result<String> {
        if messages.contains_key(&id.name) {
            Ok(id.name.clone())
        } else {
            Err(err_at(id, format!("unknown message type `{}`; is it imported?", id.name)))
        }
    };

    // Protocols: built-ins first, then the image's own.
    let mut protocols: HashMap<String, ProtocolDef> = HashMap::new();
    protocols.insert(
        STDIN.into(),
        ProtocolDef { kind: Kind::Stdin, accepts: Accepts::Only(vec![]), outputs: vec![("out".into(), vec![LINE.into()])] },
    );
    protocols.insert(STDOUT.into(), ProtocolDef { kind: Kind::Stdout, accepts: Accepts::Anything, outputs: vec![] });
    for p in &img.protocols {
        if protocols.contains_key(&p.name.name) {
            bail!(err_at(&p.name, format!("protocol `{}` is already defined", p.name.name)));
        }
        let accepts = p.inputs.iter().map(&lookup_type).collect::<Result<Vec<_>>>()?;
        let mut outputs: Vec<(String, Vec<String>)> = vec![];
        for o in &p.outputs {
            if outputs.iter().any(|(n, _)| *n == o.name.name) {
                bail!(err_at(&o.name, format!("port `{}` is declared twice", o.name.name)));
            }
            outputs.push((o.name.name.clone(), o.types.iter().map(&lookup_type).collect::<Result<_>>()?));
        }
        let module = dir.join("scripts").join(format!("{}.wasm", p.name.name));
        protocols.insert(
            p.name.name.clone(),
            ProtocolDef { kind: Kind::Wasm { protocol: p.name.name.clone(), module }, accepts: Accepts::Only(accepts), outputs },
        );
    }

    // Instance names, so wiring can refer forwards.
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, inst) in img.instances.iter().enumerate() {
        if index.insert(&inst.name.name, i).is_some() {
            bail!(err_at(&inst.name, format!("instance `{}` is declared twice", inst.name.name)));
        }
    }
    let proto_of = |inst: &image::Instance| -> Result<&ProtocolDef> {
        protocols
            .get(&inst.protocol.name)
            .ok_or_else(|| err_at(&inst.protocol, format!("unknown protocol `{}`", inst.protocol.name)))
    };

    let mut instances = vec![];
    for inst in &img.instances {
        let proto = proto_of(inst)?;
        let iname = &inst.name.name;

        if proto.kind == Kind::Stdin {
            check_line_type(&messages, &inst.protocol)?;
        }

        // Wiring: every declared port exactly once, every target accepts its types.
        let mut ports: Vec<Port> = proto
            .outputs
            .iter()
            .map(|(n, t)| Port { name: n.clone(), types: t.clone(), targets: vec![] })
            .collect();
        let mut wired = vec![false; ports.len()];
        for w in &inst.wires {
            let Some(pi) = ports.iter().position(|p| p.name == w.port.name) else {
                bail!(err_at(&w.port, format!("`{iname}` ({}) has no out port `{}`", inst.protocol.name, w.port.name)));
            };
            if std::mem::replace(&mut wired[pi], true) {
                bail!(err_at(&w.port, format!("port `{iname}.{}` is wired more than once", w.port.name)));
            }
            for t in &w.targets {
                let Some(&ti) = index.get(t.name.as_str()) else {
                    bail!(err_at(t, format!("unknown instance `{}`", t.name)));
                };
                if ports[pi].targets.contains(&ti) {
                    bail!(err_at(t, format!("`{}` appears twice in this fan-out", t.name)));
                }
                if let Accepts::Only(ins) = &proto_of(&img.instances[ti])?.accepts {
                    for ty in &ports[pi].types {
                        if !ins.contains(ty) {
                            bail!(err_at(
                                t,
                                format!(
                                    "port `{iname}.{}` may carry `{ty}`, but `{}` ({}) does not accept it",
                                    w.port.name, t.name, img.instances[ti].protocol.name
                                )
                            ));
                        }
                    }
                }
                ports[pi].targets.push(ti);
            }
        }
        if let Some(pi) = wired.iter().position(|w| !w) {
            bail!(err_at(&inst.name, format!("port `{iname}.{}` is not wired", ports[pi].name)));
        }

        // Startup sends: typed, every field present with a matching value.
        if proto.kind == Kind::Stdin && !inst.sends.is_empty() {
            bail!(err_at(&inst.sends[0].type_name, format!("{STDIN} cannot receive messages")));
        }
        let startup = inst
            .sends
            .iter()
            .map(|s| startup_message(&messages, s))
            .collect::<Result<Vec<_>>>()?;

        instances.push(Instance { name: iname.clone(), kind: proto.kind.clone(), ports, startup });
    }

    Ok(Program { messages, instances })
}

fn resolve_imports(img: &Image, dir: &Path) -> Result<BTreeMap<String, MessageType>> {
    let mut parsed: HashMap<PathBuf, BTreeMap<String, MessageType>> = HashMap::new();
    let mut out = BTreeMap::new();
    for imp in &img.imports {
        let mut path = dir.join(&imp.path);
        if path.extension().is_none() {
            path.set_extension("ts");
        }
        if !parsed.contains_key(&path) {
            let src = std::fs::read_to_string(&path)
                .with_context(|| format!("{}: cannot read import {}", imp.pos, path.display()))?;
            let decls = messages::parse(&src).with_context(|| format!("in {}", path.display()))?;
            parsed.insert(path.clone(), decls);
        }
        let decls = &parsed[&path];
        for name in &imp.names {
            let Some(ty) = decls.get(&name.name) else {
                bail!(err_at(name, format!("`{}` is not an exported class in {}", name.name, path.display())));
            };
            if out.insert(name.name.clone(), ty.clone()).is_some() {
                bail!(err_at(name, format!("`{}` is imported twice", name.name)));
            }
        }
    }
    Ok(out)
}

fn check_line_type(messages: &BTreeMap<String, MessageType>, at: &Ident) -> Result<()> {
    match messages.get(LINE) {
        Some(ty) if ty.fields == [("text".to_string(), FieldType::String)] => Ok(()),
        Some(_) => Err(err_at(at, format!("{STDIN} emits `{LINE}`, which must be declared as {{ text: string }}"))),
        None => Err(err_at(at, format!("{STDIN} emits `{LINE}`; import it from your messages"))),
    }
}

fn startup_message(messages: &BTreeMap<String, MessageType>, send: &image::Send) -> Result<Message> {
    let tid = &send.type_name;
    let Some(ty) = messages.get(&tid.name) else {
        bail!(err_at(tid, format!("unknown message type `{}`; is it imported?", tid.name)));
    };
    let given = send.fields.as_deref().unwrap_or(&[]);
    let mut fields = Document::new();
    for (fname, value) in given {
        let Some(fty) = ty.field(&fname.name) else {
            bail!(err_at(fname, format!("`{}` has no field `{}`", tid.name, fname.name)));
        };
        if fields.contains_key(&fname.name) {
            bail!(err_at(fname, format!("field `{}` given twice", fname.name)));
        }
        let bson = match (fty, value) {
            (FieldType::String, Value::Str(s)) => Bson::String(s.clone()),
            (FieldType::Bool, Value::Bool(b)) => Bson::Boolean(*b),
            (FieldType::F32 | FieldType::F64, Value::Float(x)) => Bson::Double(*x),
            (FieldType::F32 | FieldType::F64, Value::Int(i)) => Bson::Double(*i as f64),
            (t, Value::Int(i)) if t.int_range().is_some() => {
                let (lo, hi) = t.int_range().unwrap();
                if !(lo..=hi).contains(i) {
                    bail!(err_at(fname, format!("{i} does not fit in {t}")));
                }
                wire::int_to_bson(t, *i)
            }
            (t, v) => bail!(err_at(fname, format!("`{}.{}` is {t}, but was given {v:?}", tid.name, fname.name))),
        };
        fields.insert(fname.name.clone(), bson);
    }
    if let Some((missing, _)) = ty.fields.iter().find(|(n, _)| !fields.contains_key(n)) {
        bail!(err_at(tid, format!("`send {}` is missing field `{missing}`", tid.name)));
    }
    Ok(Message { type_name: tid.name.clone(), fields })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGES: &str = r#"
        export class Line { text: string }
        export class Echoed { text: string }
        export class Config { name: string; count: u8; ratio: f64; on: bool }
        export class Ping {}
    "#;

    /// Writes MESSAGES and `img` to a temp dir and checks the image.
    fn check_src(img: &str) -> Result<Program> {
        let dir = std::env::temp_dir().join(format!("real_talk_check_{}_{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("messages.ts"), MESSAGES).unwrap();
        let path = dir.join("test.img");
        std::fs::write(&path, img).unwrap();
        let res = load(&path);
        std::fs::remove_dir_all(&dir).ok();
        res
    }

    fn err(img: &str) -> String {
        format!("{:#}", check_src(img).unwrap_err())
    }

    const HEADER: &str = r#"
        import { Line, Echoed, Config, Ping } from "./messages"
        protocol Echo {
          in Line, Config, Ping
          out out: Echoed
        }
    "#;

    #[test]
    fn accepts_hello_world() {
        let p = check_src(&format!(
            "{HEADER}
            instance stdin : Environment.Stdin {{ out -> echo }}
            instance echo : Echo {{
              send Config {{ name: \"x\", count: 3, ratio: 2, on: true }}
              send Ping
              out -> stdout
            }}
            instance stdout : Environment.Stdout"
        ))
        .unwrap();
        assert_eq!(p.instances.len(), 3);
        assert_eq!(p.instances[0].kind, Kind::Stdin);
        assert_eq!(p.instances[0].ports[0].targets, [1]);
        assert!(matches!(&p.instances[1].kind, Kind::Wasm { protocol, module }
            if protocol == "Echo" && module.ends_with("scripts/Echo.wasm")));
        assert_eq!(p.instances[1].startup.len(), 2);
        assert_eq!(p.instances[1].startup[0].fields.get_i32("count").unwrap(), 3);
        assert_eq!(p.instances[1].startup[0].fields.get_f64("ratio").unwrap(), 2.0);
    }

    #[test]
    fn unwired_port() {
        assert!(err(&format!("{HEADER} instance echo : Echo")).contains("`echo.out` is not wired"));
    }

    #[test]
    fn doubly_wired_port() {
        let e = err(&format!("{HEADER} instance echo : Echo {{ out -> s\n out -> s }} instance s : Environment.Stdout"));
        assert!(e.contains("more than once"), "{e}");
    }

    #[test]
    fn unknown_port_and_target() {
        assert!(err(&format!("{HEADER} instance echo : Echo {{ nope -> echo }}")).contains("no out port `nope`"));
        assert!(err(&format!("{HEADER} instance echo : Echo {{ out -> ghost }}")).contains("unknown instance `ghost`"));
    }

    #[test]
    fn target_must_accept_port_types() {
        let e = err(&format!("{HEADER} instance a : Echo {{ out -> b }} instance b : Echo {{ out -> a }}"));
        assert!(e.contains("does not accept"), "{e}");
    }

    #[test]
    fn types_must_be_imported() {
        let e = err("import { Line } from \"./messages\" protocol P { in Echoed }");
        assert!(e.contains("unknown message type `Echoed`"), "{e}");
        let e = err("import { Nope } from \"./messages\"");
        assert!(e.contains("not an exported class"), "{e}");
    }

    #[test]
    fn startup_sends_are_typed() {
        let inst = |send: &str| format!("{HEADER} instance s : Environment.Stdout instance e : Echo {{ {send} out -> s }}");
        assert!(err(&inst("send Config { name: \"x\" }")).contains("missing field `count`"));
        assert!(err(&inst("send Config { name: 1, count: 1, ratio: 1, on: true }")).contains("is string"));
        assert!(err(&inst("send Config { name: \"\", count: 256, ratio: 1, on: true }")).contains("does not fit in u8"));
        assert!(err(&inst("send Ping { x: 1 }")).contains("no field `x`"));
        assert!(err(&inst("send Missing")).contains("unknown message type"));
    }

    #[test]
    fn stdin_requires_line() {
        let e = err("import { Echoed } from \"./messages\" instance i : Environment.Stdin { out -> o } instance o : Environment.Stdout");
        assert!(e.contains("import it"), "{e}");
    }
}
