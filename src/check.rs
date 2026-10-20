//! Resolves and validates a parsed image into a [`Program`] the runtime can run.

use crate::image::{self, Ident, Image, Pos, Route, Target, Value};
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
    /// Instances in image order; routes and replies refer to them by index.
    pub instances: Vec<Instance>,
    /// `(sender, type) -> targets`, flattened from the image's routing trees.
    pub routes: HashMap<(usize, String), Vec<usize>>,
    /// Things worth knowing that don't stop the image from running.
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct Instance {
    pub name: String,
    pub kind: Kind,
    /// `None` accepts any message.
    pub accepts: Option<Vec<String>>,
    pub emits: Vec<String>,
    pub replies: Vec<String>,
    pub startup: Vec<Message>,
}

impl Instance {
    pub fn accepts(&self, ty: &str) -> bool {
        self.accepts.as_ref().is_none_or(|a| a.iter().any(|t| t == ty))
    }
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

/// A protocol implemented by the runtime itself.
struct Builtin {
    name: &'static str,
    /// `None` accepts any message.
    accepts: Option<&'static [&'static str]>,
    emits: &'static [&'static str],
    replies: &'static [&'static str],
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
    Builtin { name: STDIN, accepts: Some(&[]), emits: &[LINE], replies: &[], startup: Startup::Never },
    Builtin { name: STDOUT, accepts: None, emits: &[], replies: &[], startup: Startup::Any },
    Builtin {
        name: SOCKET,
        accepts: Some(&["SocketWrite", "SocketClose"]),
        emits: &["SocketData", "SocketClosed", "Listening"],
        replies: &[],
        startup: Startup::Config("Listen"),
    },
    Builtin {
        name: FILES,
        accepts: Some(&["ReadFile"]),
        emits: &[],
        replies: &["FileContents", "FileError"],
        startup: Startup::Config("FilesRoot"),
    },
];

/// A protocol after resolution; built-ins and image-declared ones alike.
struct ProtocolDef {
    accepts: Option<Vec<String>>,
    emits: Vec<String>,
    replies: Vec<String>,
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
    let types_of = |ids: &[Ident]| -> Result<Vec<String>> {
        ids.iter()
            .map(|id| {
                if messages.contains_key(&id.name) {
                    Ok(id.name.clone())
                } else {
                    Err(err_at(id, format!("unknown message type `{}`; is it imported?", id.name)))
                }
            })
            .collect()
    };

    // Protocols: built-ins first, then the image's own.
    let strings = |ts: &[&str]| ts.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    let mut protocols: HashMap<String, ProtocolDef> = HashMap::new();
    for b in BUILTINS {
        protocols.insert(
            b.name.into(),
            ProtocolDef {
                accepts: b.accepts.map(strings),
                emits: strings(b.emits),
                replies: strings(b.replies),
                source: Source::Builtin(b),
            },
        );
    }
    for p in &img.protocols {
        if protocols.contains_key(&p.name.name) {
            bail!(err_at(&p.name, format!("protocol `{}` is already defined", p.name.name)));
        }
        let module = dir.join("scripts").join(format!("{}.wasm", p.name.name));
        protocols.insert(
            p.name.name.clone(),
            ProtocolDef {
                accepts: Some(types_of(&p.inputs)?),
                emits: types_of(&p.emits)?,
                replies: types_of(&p.replies)?,
                source: Source::Wasm { protocol: p.name.name.clone(), module },
            },
        );
    }

    // Instances.
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut instances = vec![];
    for (i, inst) in img.instances.iter().enumerate() {
        if index.insert(&inst.name.name, i).is_some() {
            bail!(err_at(&inst.name, format!("instance `{}` is declared twice", inst.name.name)));
        }
        let proto = protocols
            .get(&inst.protocol.name)
            .ok_or_else(|| err_at(&inst.protocol, format!("unknown protocol `{}`", inst.protocol.name)))?;
        let iname = &inst.name.name;

        if let Source::Builtin(b) = proto.source {
            let config = b.startup_config();
            let used = b.accepts.unwrap_or(&[]).iter().chain(b.emits).chain(b.replies).chain(&config);
            for ty in used {
                require_env_type(&messages, &env_messages, ty, &inst.protocol)?;
            }
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

        instances.push(Instance {
            name: iname.clone(),
            kind,
            accepts: proto.accepts.clone(),
            emits: proto.emits.clone(),
            replies: proto.replies.clone(),
            startup,
        });
    }

    // Routes: flatten every tree into (sender, type) -> targets.
    let mut routes = HashMap::new();
    let mut defined_at: HashMap<(usize, String), Pos> = HashMap::new();
    let mut pending: Vec<&Route> = img.routes.iter().collect();
    while let Some(route) = pending.pop() {
        let find = |id: &Ident| {
            index.get(id.name.as_str()).copied().ok_or_else(|| err_at(id, format!("unknown instance `{}`", id.name)))
        };
        let from = find(&route.sender)?;
        let sender = &instances[from];
        let types = if route.selector.name == "*" {
            if sender.emits.is_empty() {
                bail!(err_at(&route.selector, format!("`{}` doesn't emit anything", sender.name)));
            }
            sender.emits.clone()
        } else if !sender.emits.contains(&route.selector.name) {
            bail!(err_at(
                &route.selector,
                format!("`{}` doesn't emit `{}` (it emits: {})", sender.name, route.selector.name, list(&sender.emits))
            ));
        } else {
            vec![route.selector.name.clone()]
        };

        let mut targets = vec![];
        for t in &route.targets {
            let id = t.instance();
            let to = find(id)?;
            if let Target::Leaf(_) = t
                && targets.contains(&to)
            {
                bail!(err_at(id, format!("`{}` appears twice in this route", id.name)));
            }
            if !targets.contains(&to) {
                targets.push(to);
            }
            if let Target::Node(child) = t {
                pending.push(child);
            }
            let target = &instances[to];
            for ty in &types {
                if !target.accepts(ty) {
                    bail!(err_at(id, format!("`{}` doesn't accept `{ty}`, which `{}` emits", target.name, sender.name)));
                }
            }
            // Whatever the target replies with goes back to the sender.
            for ty in &target.replies {
                if !sender.accepts(ty) {
                    bail!(err_at(
                        id,
                        format!("`{}` replies with `{ty}`, which `{}` doesn't accept", target.name, sender.name)
                    ));
                }
            }
        }

        for ty in types {
            if let Some(prev) = defined_at.insert((from, ty.clone()), route.selector.pos) {
                bail!(err_at(
                    &route.selector,
                    format!("`{}.{ty}` is already routed at {prev}; list every target in one place", sender.name)
                ));
            }
            routes.insert((from, ty), targets.clone());
        }
    }

    let mut warnings = vec![];
    for (i, inst) in instances.iter().enumerate() {
        for ty in &inst.emits {
            if !routes.contains_key(&(i, ty.clone())) {
                warnings.push(format!("`{}.{ty}` is not routed; those messages will be dropped", inst.name));
            }
        }
    }

    Ok(Program { messages, instances, routes, warnings })
}

fn list(types: &[String]) -> String {
    if types.is_empty() { "nothing".into() } else { types.join(", ") }
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
          emits Echoed
        }
        protocol Answerer {
          in Ping
          replies Echoed
        }
        instance stdin : Environment.Stdin
        instance echo : Echo
        instance stdout : Environment.Stdout
    "#;

    fn route(p: &Program, from: &str, ty: &str) -> Vec<String> {
        let i = p.instances.iter().position(|x| x.name == from).unwrap();
        p.routes[&(i, ty.to_string())].iter().map(|&t| p.instances[t].name.clone()).collect()
    }

    #[test]
    fn accepts_hello_world() {
        let p = check_src(&format!("{HEADER} stdin.Line {{ echo.Echoed {{ stdout }} }}")).unwrap();
        assert_eq!(p.instances.len(), 3);
        assert_eq!(p.instances[0].kind, Kind::Stdin);
        assert!(matches!(&p.instances[1].kind, Kind::Wasm { protocol, module }
            if protocol == "Echo" && module.ends_with("scripts/Echo.wasm")));
        assert_eq!(route(&p, "stdin", "Line"), ["echo"]);
        assert_eq!(route(&p, "echo", "Echoed"), ["stdout"]);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    }

    #[test]
    fn nested_routes_flatten_and_fan_out() {
        let p = check_src(&format!(
            "{HEADER} instance echo2 : Echo
            stdin.Line {{ echo.* {{ stdout }}, echo2.Echoed {{ stdout }}, stdout }}"
        ))
        .unwrap();
        assert_eq!(route(&p, "stdin", "Line"), ["echo", "echo2", "stdout"]);
        assert_eq!(route(&p, "echo", "Echoed"), ["stdout"]);
        assert_eq!(route(&p, "echo2", "Echoed"), ["stdout"]);
    }

    #[test]
    fn unrouted_emits_are_warnings_not_errors() {
        let p = check_src(&format!("{HEADER} stdin.Line {{ echo }}")).unwrap();
        assert_eq!(p.warnings, ["`echo.Echoed` is not routed; those messages will be dropped"]);
    }

    #[test]
    fn a_sender_and_type_is_routed_once() {
        let e = err(&format!("{HEADER} stdin.Line {{ echo }} stdin.Line {{ stdout }}"));
        assert!(e.contains("`stdin.Line` is already routed at"), "{e}");
        let e = err(&format!("{HEADER} stdin.Line {{ echo.* {{ stdout }} }} echo.Echoed {{ stdout }}"));
        assert!(e.contains("`echo.Echoed` is already routed"), "{e}");
        let e = err(&format!("{HEADER} stdin.Line {{ echo, echo }}"));
        assert!(e.contains("appears twice"), "{e}");
    }

    #[test]
    fn routes_must_match_emits_and_accepts() {
        assert!(err(&format!("{HEADER} stdin.Echoed {{ echo }}")).contains("`stdin` doesn't emit `Echoed` (it emits: Line)"));
        assert!(err(&format!("{HEADER} stdout.* {{ echo }}")).contains("`stdout` doesn't emit anything"));
        assert!(err(&format!("{HEADER} echo.Echoed {{ echo }}")).contains("`echo` doesn't accept `Echoed`"));
        assert!(err(&format!("{HEADER} ghost.Line {{ echo }}")).contains("unknown instance `ghost`"));
        assert!(err(&format!("{HEADER} stdin.Line {{ ghost }}")).contains("unknown instance `ghost`"));
    }

    #[test]
    fn replies_must_be_accepted_by_the_sender() {
        let img = format!(
            "{HEADER}
            protocol Pinger {{ in Line  emits Ping }}
            instance pinger : Pinger
            instance answerer : Answerer
            stdin.Line {{ pinger.Ping {{ answerer }} }}"
        );
        let e = err(&img);
        assert!(e.contains("`answerer` replies with `Echoed`, which `pinger` doesn't accept"), "{e}");
        assert!(check_src(&img.replace("in Line  emits Ping", "in Line, Echoed  emits Ping")).is_ok());
    }

    #[test]
    fn types_must_be_imported() {
        let e = err("import { Line } from \"./messages\" protocol P { in Echoed }");
        assert!(e.contains("unknown message type `Echoed`"), "{e}");
        let e = err("import { Line } from \"./messages\" protocol P { emits Echoed }");
        assert!(e.contains("unknown message type `Echoed`"), "{e}");
        let e = err("import { Nope } from \"./messages\"");
        assert!(e.contains("not an exported class"), "{e}");
    }

    #[test]
    fn startup_sends_are_typed() {
        let inst = |send: &str| format!("{HEADER} instance e : Echo {{ {send} }}");
        let p = check_src(&inst("send Config { name: \"x\", count: 3, ratio: 2, on: true } send Ping")).unwrap();
        let e = &p.instances[3];
        assert_eq!(e.startup.len(), 2);
        assert_eq!(e.startup[0].fields.get_i32("count").unwrap(), 3);
        assert_eq!(e.startup[0].fields.get_f64("ratio").unwrap(), 2.0);
        assert!(err(&inst("send Config { name: \"x\" }")).contains("missing field `count`"));
        assert!(err(&inst("send Config { name: 1, count: 1, ratio: 1, on: true }")).contains("is string"));
        assert!(err(&inst("send Config { name: \"\", count: 256, ratio: 1, on: true }")).contains("does not fit in u8"));
        assert!(err(&inst("send Ping { x: 1 }")).contains("no field `x`"));
        assert!(err(&inst("send Missing")).contains("unknown message type"));
        assert!(err(&inst("send Blob { data: 1 }")).contains("is ArrayBuffer"));
    }

    #[test]
    fn bytes_fields_take_string_literals_as_utf8() {
        let p = check_src(&format!("{HEADER} instance e : Echo {{ send Blob {{ data: \"hé\" }} }}")).unwrap();
        let data = p.instances[3].startup[0].fields.get_binary_generic("data").unwrap();
        assert_eq!(data, "hé".as_bytes());
    }

    const SERVER_IMG: &str = r#"
        import { Listen, Listening, SocketData, SocketClosed, SocketWrite, SocketClose,
                 FilesRoot, ReadFile, FileContents, FileError } from "real-talk/env"
        protocol Handler {
          in SocketData, SocketClosed, FileContents, FileError
          emits ReadFile
          replies SocketWrite, SocketClose
        }
        instance h : Handler
        instance log : Environment.Stdout
    "#;

    #[test]
    fn socket_and_files_take_config_from_the_image() {
        let p = check_src(&format!(
            r#"{SERVER_IMG}
            instance net : Environment.Socket {{ send Listen {{ address: "127.0.0.1:0" }} }}
            instance fs : Environment.Files {{ send FilesRoot {{ path: "public" }} }}
            net.SocketData {{ h.ReadFile {{ fs }} }}
            net.SocketClosed {{ h }}
            net.Listening {{ log }}"#
        ))
        .unwrap();
        assert_eq!(p.instances[2].kind, Kind::Socket { address: "127.0.0.1:0".into() });
        assert!(p.instances[2].startup.is_empty(), "config is consumed by the checker");
        assert!(matches!(&p.instances[3].kind, Kind::Files { root } if root.ends_with("public")));
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    }

    #[test]
    fn config_is_required_exactly_once() {
        let fs = "instance fs : Environment.Files { send FilesRoot { path: \".\" } }";
        let e = err(&format!("{SERVER_IMG} {fs} instance net : Environment.Socket"));
        assert!(e.contains("needs exactly one startup message, `send Listen"), "{e}");
        let e = err(&format!(
            "{SERVER_IMG} {fs} instance net : Environment.Socket {{ send Listen {{ address: \"a\" }} send Listen {{ address: \"b\" }} }}"
        ));
        assert!(e.contains("exactly one"), "{e}");
    }

    #[test]
    fn config_cannot_be_sent_at_runtime() {
        let e = err(
            r#"import { FilesRoot, ReadFile, FileContents, FileError } from "real-talk/env"
            protocol Sneaky { emits FilesRoot }
            instance s : Sneaky
            instance fs : Environment.Files { send FilesRoot { path: "." } }
            s.FilesRoot { fs }"#,
        );
        assert!(e.contains("`fs` doesn't accept `FilesRoot`"), "{e}");
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
            instance net : Environment.Socket { send Listen { port: 1 } }"#,
        )
        .unwrap();
        let e = format!("{:#}", check(&img, &dir).unwrap_err());
        std::fs::remove_dir_all(&dir).ok();
        assert!(e.contains("`Listen`, which must be declared as { address: string }"), "{e}");
    }

    #[test]
    fn stdin_requires_line() {
        let e = err("import { Echoed } from \"./messages\" instance i : Environment.Stdin");
        assert!(e.contains("import it"), "{e}");
    }
}
