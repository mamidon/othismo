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
pub const SOCKET: &str = "Environment.Socket";
pub const FILES: &str = "Environment.Files";

/// The import path for environment message types, and their declarations.
pub const ENV_MODULE: &str = "real-talk/env";
const ENV_SOURCE: &str = include_str!("../as/env.ts");

/// The message type `Environment.Stdin` emits.
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
    /// Listens on `address` (from the image's `send Listen`).
    Socket { address: String },
    /// Serves files under `root` (from `send FilesRoot`, relative to the image).
    Files { root: PathBuf },
}

#[derive(Debug)]
pub struct Port {
    pub name: String,
    pub types: Vec<String>,
    pub targets: Vec<usize>,
}

/// A protocol implemented by the runtime itself.
struct Builtin {
    name: &'static str,
    /// `None` accepts any message.
    accepts: Option<&'static [&'static str]>,
    outputs: &'static [(&'static str, &'static [&'static str])],
    startup: Startup,
}

#[derive(Clone, Copy, PartialEq)]
enum Startup {
    /// No startup sends allowed.
    Never,
    /// Startup sends of any type (delivered like normal messages).
    Any,
    /// Exactly one send of this type, consumed by the checker as config.
    Config(&'static str),
}

const BUILTINS: &[Builtin] = &[
    Builtin { name: STDIN, accepts: Some(&[]), outputs: &[("out", &[LINE])], startup: Startup::Never },
    Builtin { name: STDOUT, accepts: None, outputs: &[], startup: Startup::Any },
    Builtin {
        name: SOCKET,
        accepts: Some(&["SocketWrite", "SocketClose"]),
        outputs: &[("out", &["SocketData", "SocketClosed"]), ("events", &["Listening"])],
        startup: Startup::Config("Listen"),
    },
    Builtin {
        name: FILES,
        accepts: Some(&["ReadFile"]),
        outputs: &[("out", &["FileContents", "FileError"])],
        startup: Startup::Config("FilesRoot"),
    },
];

/// A protocol after resolution; built-ins and image-declared ones alike.
struct ProtocolDef {
    /// `None` accepts any message.
    accepts: Option<Vec<String>>,
    outputs: Vec<(String, Vec<String>)>,
    source: Source,
}

enum Source {
    Builtin(&'static Builtin),
    Wasm { protocol: String, module: PathBuf },
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

/// Validates `img`; relative paths (imports, `scripts/`, `FilesRoot`) resolve against `dir`.
pub fn check(img: &Image, dir: &Path) -> Result<Program> {
    let messages = resolve_imports(img, dir)?;
    let env_messages = messages::parse(ENV_SOURCE).expect("as/env.ts parses");
    let lookup_type = |id: &Ident| -> Result<String> {
        if messages.contains_key(&id.name) {
            Ok(id.name.clone())
        } else {
            Err(err_at(id, format!("unknown message type `{}`; is it imported?", id.name)))
        }
    };

    // Protocols: built-ins first, then the image's own.
    let mut protocols: HashMap<String, ProtocolDef> = HashMap::new();
    for b in BUILTINS {
        let strings = |ts: &[&str]| ts.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        protocols.insert(
            b.name.into(),
            ProtocolDef {
                accepts: b.accepts.map(strings),
                outputs: b.outputs.iter().map(|(n, ts)| (n.to_string(), strings(ts))).collect(),
                source: Source::Builtin(b),
            },
        );
    }
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
            ProtocolDef { accepts: Some(accepts), outputs, source: Source::Wasm { protocol: p.name.name.clone(), module } },
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

        if let Source::Builtin(b) = proto.source {
            let config = b.startup_config();
            for ty in b.accepts.unwrap_or(&[]).iter().chain(b.outputs.iter().flat_map(|(_, ts)| *ts)).chain(&config) {
                require_env_type(&messages, &env_messages, ty, &inst.protocol)?;
            }
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
                if let Some(ins) = &proto_of(&img.instances[ti])?.accepts {
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
        let mut startup = inst
            .sends
            .iter()
            .map(|s| startup_message(&messages, s))
            .collect::<Result<Vec<_>>>()?;

        let kind = match &proto.source {
            Source::Wasm { protocol, module } => Kind::Wasm { protocol: protocol.clone(), module: module.clone() },
            Source::Builtin(b) => {
                let config = match b.startup {
                    Startup::Never if !inst.sends.is_empty() => {
                        bail!(err_at(&inst.sends[0].type_name, format!("{} cannot receive startup messages", b.name)))
                    }
                    Startup::Never | Startup::Any => None,
                    Startup::Config(ty) => {
                        if inst.sends.len() != 1 || inst.sends[0].type_name.name != ty {
                            bail!(err_at(
                                &inst.name,
                                format!("{} needs exactly one startup message, `send {ty} {{ ... }}`", b.name)
                            ));
                        }
                        Some(startup.remove(0))
                    }
                };
                builtin_kind(b, config.as_ref(), dir)
            }
        };

        instances.push(Instance { name: iname.clone(), kind, ports, startup });
    }

    Ok(Program { messages, instances })
}

impl Builtin {
    fn startup_config(&self) -> Option<&'static str> {
        match self.startup {
            Startup::Config(ty) => Some(ty),
            _ => None,
        }
    }
}

fn builtin_kind(b: &Builtin, config: Option<&Message>, dir: &Path) -> Kind {
    let config_str = |key: &str| config.and_then(|m| m.fields.get_str(key).ok()).unwrap_or_default().to_string();
    match b.name {
        STDIN => Kind::Stdin,
        STDOUT => Kind::Stdout,
        SOCKET => Kind::Socket { address: config_str("address") },
        FILES => Kind::Files { root: dir.join(config_str("path")) },
        other => unreachable!("unknown builtin {other}"),
    }
}

fn resolve_imports(img: &Image, dir: &Path) -> Result<BTreeMap<String, MessageType>> {
    let mut parsed: HashMap<PathBuf, BTreeMap<String, MessageType>> = HashMap::new();
    let mut out = BTreeMap::new();
    for imp in &img.imports {
        let path = if imp.path == ENV_MODULE {
            PathBuf::from(ENV_MODULE)
        } else if imp.path.starts_with('.') || imp.path.starts_with('/') {
            let mut path = dir.join(&imp.path);
            if path.extension().is_none() {
                path.set_extension("ts");
            }
            path
        } else {
            bail!("{}: unknown module \"{}\"; use a relative path or \"{ENV_MODULE}\"", imp.pos, imp.path);
        };
        if !parsed.contains_key(&path) {
            let src = if imp.path == ENV_MODULE {
                ENV_SOURCE.to_string()
            } else {
                std::fs::read_to_string(&path).with_context(|| format!("{}: cannot read import {}", imp.pos, path.display()))?
            };
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

/// An environment protocol uses `ty`, so the image must import it, with the
/// fields `as/env.ts` declares (whether or not it came from there).
fn require_env_type(
    messages: &BTreeMap<String, MessageType>,
    env: &BTreeMap<String, MessageType>,
    ty: &str,
    at: &Ident,
) -> Result<()> {
    let want = &env[ty];
    match messages.get(ty) {
        Some(have) if have.fields == want.fields => Ok(()),
        Some(_) => {
            let fields: Vec<String> = want.fields.iter().map(|(n, t)| format!("{n}: {t}")).collect();
            Err(err_at(at, format!("{} uses `{ty}`, which must be declared as {{ {} }}", at.name, fields.join(", "))))
        }
        None => Err(err_at(at, format!("{} uses `{ty}`; import it from \"{ENV_MODULE}\"", at.name))),
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
            (FieldType::Bytes, Value::Str(s)) => wire::bytes(s.as_bytes().to_vec()),
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
        export class Blob { data: ArrayBuffer }
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
        import { Line, Echoed, Config, Ping, Blob } from "./messages"
        protocol Echo {
          in Line, Config, Ping, Blob
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
        assert!(err(&inst("send Blob { data: 1 }")).contains("is ArrayBuffer"));
    }

    #[test]
    fn bytes_fields_take_string_literals_as_utf8() {
        let p = check_src(&format!("{HEADER} instance s : Environment.Stdout instance e : Echo {{ send Blob {{ data: \"hé\" }} out -> s }}")).unwrap();
        let data = p.instances[1].startup[0].fields.get_binary_generic("data").unwrap();
        assert_eq!(data, "hé".as_bytes());
    }

    const SOCKET_IMG: &str = r#"
        import { Listen, Listening, SocketData, SocketClosed, SocketWrite, SocketClose,
                 FilesRoot, ReadFile, FileContents, FileError } from "real-talk/env"
        protocol Handler {
          in SocketData, SocketClosed, FileContents, FileError
          out net: SocketWrite, SocketClose
          out files: ReadFile
        }
        instance h : Handler { net -> net  files -> fs }
        instance log : Environment.Stdout
    "#;

    #[test]
    fn socket_and_files_take_config_from_the_image() {
        let p = check_src(&format!(
            r#"{SOCKET_IMG}
            instance net : Environment.Socket {{ send Listen {{ address: "127.0.0.1:0" }}  out -> h  events -> log }}
            instance fs : Environment.Files {{ send FilesRoot {{ path: "public" }}  out -> h }}"#
        ))
        .unwrap();
        assert_eq!(p.instances[2].kind, Kind::Socket { address: "127.0.0.1:0".into() });
        assert!(p.instances[2].startup.is_empty(), "config is consumed by the checker");
        assert!(matches!(&p.instances[3].kind, Kind::Files { root } if root.ends_with("public")));
    }

    #[test]
    fn config_is_required_exactly_once() {
        let net = "instance fs : Environment.Files { send FilesRoot { path: \".\" }  out -> h }";
        let e = err(&format!("{SOCKET_IMG} {net} instance net : Environment.Socket {{ out -> h  events -> log }}"));
        assert!(e.contains("needs exactly one startup message, `send Listen"), "{e}");
        let e = err(&format!(
            "{SOCKET_IMG} {net} instance net : Environment.Socket {{
               send Listen {{ address: \"a\" }} send Listen {{ address: \"b\" }} out -> h  events -> log }}"
        ));
        assert!(e.contains("exactly one"), "{e}");
    }

    #[test]
    fn config_cannot_be_wired_at_runtime() {
        let e = err(
            r#"import { FilesRoot, ReadFile, FileContents, FileError } from "real-talk/env"
            protocol Sneaky { out x: FilesRoot }
            instance s : Sneaky { x -> fs }
            instance fs : Environment.Files { send FilesRoot { path: "." }  out -> log }
            instance log : Environment.Stdout"#,
        );
        assert!(e.contains("does not accept"), "{e}");
    }

    #[test]
    fn env_types_must_match_their_declaration() {
        // A user-declared Line with the right shape is fine (see accepts_hello_world);
        // a Listen of the wrong shape is not.
        let dir = std::env::temp_dir().join(format!("real_talk_envshape_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.ts"), "export class Listen { port: i32 }").unwrap();
        let img = image::parse(
            r#"import { Listen } from "./m"
            import { Listening, SocketData, SocketClosed, SocketWrite, SocketClose } from "real-talk/env"
            instance net : Environment.Socket { send Listen { port: 1 } out -> o events -> o }
            instance o : Environment.Stdout"#,
        )
        .unwrap();
        let e = format!("{:#}", check(&img, &dir).unwrap_err());
        std::fs::remove_dir_all(&dir).ok();
        assert!(e.contains("`Listen`, which must be declared as { address: string }"), "{e}");
    }

    #[test]
    fn stdin_requires_line() {
        let e = err("import { Echoed } from \"./messages\" instance i : Environment.Stdin { out -> o } instance o : Environment.Stdout");
        assert!(e.contains("import it"), "{e}");
    }
}
