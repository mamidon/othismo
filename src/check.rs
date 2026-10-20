//! Resolves and validates a parsed image into a [`Program`] the runtime can run.

use crate::image::{self, Ident, Image, Value};
use crate::messages::{self, FieldType, MessageType};
use crate::ns::{self, Verb};
use crate::wire::{self, Message};
use anyhow::{Context, Result, bail};
use bson::{Bson, Document};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::path::{Path, PathBuf};

pub const CONSOLE: &str = "Environment.Console";
pub const TCP: &str = "Environment.Tcp";
pub const FILES: &str = "Environment.Files";

/// The import path for the runtime's message types, and their declarations.
/// These types are always in scope in an image; importing them is optional.
pub const ENV_MODULE: &str = "real-talk/env";
const ENV_SOURCE: &str = include_str!("../as/env.ts");

#[derive(Debug)]
pub struct Program {
    /// Every message type the image knows: the runtime's plus its imports.
    pub messages: BTreeMap<String, MessageType>,
    /// Instances in image order; everything else refers to them by index.
    pub instances: Vec<Instance>,
}

#[derive(Debug)]
pub struct Instance {
    pub name: String,
    pub kind: Kind,
    /// The initial view.
    pub mounts: Vec<MountDecl>,
    /// Requests the image makes of this instance at startup.
    pub startup: Vec<Startup>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MountDecl {
    /// Clean, absolute.
    pub at: String,
    pub target: usize,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Startup {
    pub verb: Verb,
    /// Clean, in the instance's served tree.
    pub path: String,
    pub body: Option<Message>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `instance x : Name` runs `scripts/Name.wasm`.
    Wasm { type_name: String, module: PathBuf },
    Console,
    /// Listens on `address` (from the image's `send Listen`).
    Tcp { address: String },
    /// Serves files under `root` (from `send FilesRoot`, relative to the image).
    Files { root: PathBuf },
}

/// An environment type and the one config message it takes, if any.
const BUILTINS: &[(&str, Option<&str>)] = &[(CONSOLE, None), (TCP, Some("Listen")), (FILES, Some("FilesRoot"))];

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

/// The runtime's own message types (`as/env.ts`).
pub fn env_messages() -> BTreeMap<String, MessageType> {
    messages::parse(ENV_SOURCE).expect("as/env.ts parses")
}

/// Validates `img`; relative paths (imports, `scripts/`, `FilesRoot`) resolve against `dir`.
pub fn check(img: &Image, dir: &Path) -> Result<Program> {
    let messages = resolve_imports(img, dir)?;

    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, inst) in img.instances.iter().enumerate() {
        if index.insert(&inst.name.name, i).is_some() {
            bail!(err_at(&inst.name, format!("instance `{}` is declared twice", inst.name.name)));
        }
    }

    let mut instances = vec![];
    for inst in &img.instances {
        let ty = &inst.type_name;
        let builtin = BUILTINS.iter().find(|(name, _)| *name == ty.name);
        let kind = match builtin {
            Some(&(name, config_type)) => {
                if let Some(m) = inst.mounts.first() {
                    bail!(err_at(&m.target, format!("{name} has no view to mount into; it only serves")));
                }
                let config = match (config_type, inst.config.as_slice()) {
                    (None, []) => None,
                    (None, [c, ..]) => bail!(err_at(&c.type_name, format!("{name} takes no config"))),
                    (Some(t), [c]) if c.type_name.name == t => Some(body_message(&messages, c)?),
                    (Some(t), _) => {
                        bail!(err_at(&inst.name, format!("{name} needs exactly one config message, `send {t} {{ ... }}`")))
                    }
                };
                builtin_kind(name, config.as_ref(), dir)
            }
            None if ty.name.starts_with("Environment.") => {
                let known: Vec<&str> = BUILTINS.iter().map(|(n, _)| *n).collect();
                bail!(err_at(ty, format!("unknown environment type `{}` (there are: {})", ty.name, known.join(", "))));
            }
            None if ty.name.contains('.') => bail!(err_at(ty, format!("`{}` is not a module name", ty.name))),
            None => {
                if let Some(c) = inst.config.first() {
                    bail!(err_at(
                        &c.type_name,
                        "`send` configures environment objects; use a startup request such as `write /path T { ... }`"
                    ));
                }
                Kind::Wasm { type_name: ty.name.clone(), module: dir.join("scripts").join(format!("{}.wasm", ty.name)) }
            }
        };

        let mut mounts: Vec<MountDecl> = vec![];
        for m in &inst.mounts {
            let target = *index
                .get(m.target.name.as_str())
                .ok_or_else(|| err_at(&m.target, format!("unknown instance `{}`", m.target.name)))?;
            let at = ns::mount_point(&m.at.path)
                .map_err(|_| anyhow::anyhow!("{}: `{}` is not a clean absolute path", m.at.pos, m.at.path))?;
            if mounts.iter().any(|prev| prev.at == at) {
                bail!("{}: `{at}` is already mounted in `{}`'s view", m.at.pos, inst.name.name);
            }
            mounts.push(MountDecl { at, target, read_only: m.read_only });
        }

        let mut startup = vec![];
        for r in &inst.requests {
            let path = ns::clean(&r.path.path)
                .map_err(|_| anyhow::anyhow!("{}: `{}` is not a clean absolute path", r.path.pos, r.path.path))?;
            let body = r.body.as_ref().map(|b| body_message(&messages, b)).transpose()?;
            startup.push(Startup { verb: r.verb, path, body });
        }

        instances.push(Instance { name: inst.name.name.clone(), kind, mounts, startup });
    }

    Ok(Program { messages, instances })
}

/// A summary of each instance's view and the most it can ever reach, for `--check`.
pub fn describe(p: &Program) -> String {
    let mounts: Vec<Vec<(usize, bool)>> =
        p.instances.iter().map(|i| i.mounts.iter().map(|m| (m.target, m.read_only)).collect()).collect();
    let reach = ns::reach(&mounts);
    let width = p.instances.iter().map(|i| i.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (i, inst) in p.instances.iter().enumerate() {
        if inst.mounts.is_empty() {
            writeln!(out, "{:width$}  sees nothing", inst.name).unwrap();
            continue;
        }
        let sees: Vec<String> = inst
            .mounts
            .iter()
            .map(|m| format!("{} → {}{}", m.at, p.instances[m.target].name, if m.read_only { " ro" } else { "" }))
            .collect();
        writeln!(out, "{:width$}  sees {}", inst.name, sees.join(", ")).unwrap();
        let reaches: Vec<String> = reach[i]
            .iter()
            .map(|(&t, r)| {
                let mode = if r.read_only { "ro" } else { "rw" };
                match r.via {
                    Some(v) => format!("{} {mode} (via {})", p.instances[t].name, p.instances[v].name),
                    None => format!("{} {mode}", p.instances[t].name),
                }
            })
            .collect();
        writeln!(out, "{:width$}  can reach at most: {}", "", reaches.join(", ")).unwrap();
    }
    out
}

fn builtin_kind(name: &str, config: Option<&Message>, dir: &Path) -> Kind {
    let config_str = |key: &str| config.and_then(|m| m.fields.get_str(key).ok()).unwrap_or_default().to_string();
    match name {
        CONSOLE => Kind::Console,
        TCP => Kind::Tcp { address: config_str("address") },
        FILES => Kind::Files { root: dir.join(config_str("path")) },
        other => unreachable!("unknown builtin {other}"),
    }
}

fn resolve_imports(img: &Image, dir: &Path) -> Result<BTreeMap<String, MessageType>> {
    let mut parsed: HashMap<PathBuf, BTreeMap<String, MessageType>> = HashMap::new();
    let env = env_messages();
    let mut out = env.clone();
    let mut imported: HashMap<String, PathBuf> = HashMap::new();
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
            if imported.insert(name.name.clone(), path.clone()).is_some() {
                bail!(err_at(name, format!("`{}` is imported twice", name.name)));
            }
            // The runtime's own types may be redeclared, but only exactly.
            if let Some(want) = env.get(&name.name)
                && want.fields != ty.fields
            {
                let fields: Vec<String> = want.fields.iter().map(|(n, t)| format!("{n}: {t}")).collect();
                bail!(err_at(
                    name,
                    format!("`{}` is a runtime type and must be declared as {{ {} }}", name.name, fields.join(", "))
                ));
            }
            out.insert(name.name.clone(), ty.clone());
        }
    }
    Ok(out)
}

fn body_message(messages: &BTreeMap<String, MessageType>, body: &image::Body) -> Result<Message> {
    let tid = &body.type_name;
    let Some(ty) = messages.get(&tid.name) else {
        bail!(err_at(tid, format!("unknown message type `{}`; is it imported?", tid.name)));
    };
    let given = body.fields.as_deref().unwrap_or(&[]);
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
        bail!(err_at(tid, format!("`{}` is missing field `{missing}`", tid.name)));
    }
    Ok(Message { type_name: tid.name.clone(), fields })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGES: &str = r#"
        export class Greeting { text: string }
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
        import { Greeting, Config, Ping, Blob } from "./messages"
        instance cons : Environment.Console
    "#;

    #[test]
    fn accepts_hello_world() {
        let p = check_src(&format!("{HEADER} instance echo : Echo {{ mount cons /dev/cons }}")).unwrap();
        assert_eq!(p.instances.len(), 2);
        assert_eq!(p.instances[0].kind, Kind::Console);
        assert!(matches!(&p.instances[1].kind, Kind::Wasm { type_name, module }
            if type_name == "Echo" && module.ends_with("scripts/Echo.wasm")));
        assert_eq!(p.instances[1].mounts, [MountDecl { at: "/dev/cons".into(), target: 0, read_only: false }]);
        assert!(p.messages.contains_key("Line"), "runtime types are always in scope");
        assert!(p.messages.contains_key("Greeting"));
    }

    #[test]
    fn mounts_are_checked() {
        assert!(err(&format!("{HEADER} instance a : A {{ mount ghost /g }}")).contains("unknown instance `ghost`"));
        let e = err(&format!("{HEADER} instance a : A {{ mount cons /x/../y }}"));
        assert!(e.contains("`/x/../y` is not a clean absolute path"), "{e}");
        let e = err(&format!("{HEADER} instance a : A {{ mount cons /c mount cons /c/ }}"));
        assert!(e.contains("`/c` is already mounted"), "{e}");
        let e = err(&format!("{HEADER} instance net : Environment.Console {{ mount cons /c }}"));
        assert!(e.contains("only serves"), "{e}");
        // Forward references are fine: views are built once every instance exists.
        let p = check_src(&format!("{HEADER} instance a : A {{ mount b /b readonly }} instance b : B")).unwrap();
        assert_eq!(p.instances[1].mounts[0], MountDecl { at: "/b".into(), target: 2, read_only: true });
    }

    #[test]
    fn startup_requests_are_typed() {
        let inst = |req: &str| format!("{HEADER} instance e : Echo {{ {req} }}");
        let p = check_src(&inst(
            "write /config Config { name: \"x\", count: 3, ratio: 2, on: true } invoke / Ping remove /a//b/",
        ))
        .unwrap();
        let s = &p.instances[1].startup;
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].verb, s[0].path.as_str()), (Verb::Write, "/config"));
        let body = s[0].body.as_ref().unwrap();
        assert_eq!(body.fields.get_i32("count").unwrap(), 3);
        assert_eq!(body.fields.get_f64("ratio").unwrap(), 2.0);
        assert_eq!(s[1].body.as_ref().unwrap().type_name, "Ping");
        assert_eq!((s[2].verb, s[2].path.as_str(), &s[2].body), (Verb::Remove, "/a/b/", &None));

        assert!(err(&inst("write /c Config { name: \"x\" }")).contains("missing field `count`"));
        assert!(err(&inst("write /c Config { name: 1, count: 1, ratio: 1, on: true }")).contains("is string"));
        assert!(err(&inst("write /c Config { name: \"\", count: 256, ratio: 1, on: true }")).contains("does not fit in u8"));
        assert!(err(&inst("invoke / Ping { x: 1 }")).contains("no field `x`"));
        assert!(err(&inst("invoke / Missing")).contains("unknown message type"));
        assert!(err(&inst("write / Blob { data: 1 }")).contains("is ArrayBuffer"));
        assert!(err(&inst("read /../x")).contains("not a clean absolute path"));
        assert!(err(&inst("send Ping")).contains("use a startup request"));
    }

    #[test]
    fn bytes_fields_take_string_literals_as_utf8() {
        let p = check_src(&format!("{HEADER} instance e : Echo {{ write / Blob {{ data: \"hé\" }} }}")).unwrap();
        let data = p.instances[1].startup[0].body.as_ref().unwrap().fields.get_binary_generic("data").unwrap();
        assert_eq!(data, "hé".as_bytes());
    }

    #[test]
    fn environment_objects_take_config_from_the_image() {
        let p = check_src(
            r#"instance net : Environment.Tcp { send Listen { address: "127.0.0.1:0" } }
            instance fs : Environment.Files { send FilesRoot { path: "public" } }"#,
        )
        .unwrap();
        assert_eq!(p.instances[0].kind, Kind::Tcp { address: "127.0.0.1:0".into() });
        assert!(matches!(&p.instances[1].kind, Kind::Files { root } if root.ends_with("public")));
        assert!(p.instances[0].startup.is_empty(), "config isn't a request");

        let e = err("instance net : Environment.Tcp");
        assert!(e.contains("needs exactly one config message, `send Listen"), "{e}");
        let e = err(r#"instance net : Environment.Tcp { send Listen { address: "a" } send Listen { address: "b" } }"#);
        assert!(e.contains("exactly one"), "{e}");
        let e = err(r#"instance c : Environment.Console { send Listen { address: "a" } }"#);
        assert!(e.contains("takes no config"), "{e}");
        let e = err("instance c : Environment.Stdin");
        assert!(e.contains("unknown environment type `Environment.Stdin`"), "{e}");
    }

    #[test]
    fn runtime_types_can_only_be_redeclared_exactly() {
        let dir = std::env::temp_dir().join(format!("real_talk_envshape_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.ts"), "export class Listen { port: i32 }\nexport class Line { text: string }").unwrap();
        let ok = image::parse(r#"import { Line } from "./m" import { Text } from "real-talk/env""#).unwrap();
        assert!(check(&ok, &dir).is_ok());
        let bad = image::parse(r#"import { Listen } from "./m""#).unwrap();
        let e = format!("{:#}", check(&bad, &dir).unwrap_err());
        let missing = image::parse(r#"import { Nope } from "real-talk/env""#).unwrap();
        let e2 = format!("{:#}", check(&missing, &dir).unwrap_err());
        std::fs::remove_dir_all(&dir).ok();
        assert!(e.contains("`Listen` is a runtime type and must be declared as { address: string }"), "{e}");
        assert!(e2.contains("not an exported class"), "{e2}");
    }

    #[test]
    fn describes_views_and_reach() {
        let p = check_src(
            r#"instance cons : Environment.Console
            instance files : Environment.Files { send FilesRoot { path: "." } }
            instance site : Site { mount files /files readonly }
            instance http : Http { mount site /www  mount cons /dev/cons }"#,
        )
        .unwrap();
        assert_eq!(
            describe(&p),
            "cons   sees nothing\n\
             files  sees nothing\n\
             site   sees /files → files ro\n       can reach at most: files ro\n\
             http   sees /www → site, /dev/cons → cons\n       can reach at most: cons rw, files ro (via site), site rw\n"
        );
    }
}
