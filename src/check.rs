//! Resolves and validates a parsed image into a [`Program`] the runtime can run.

use crate::image::{self, Ident, Image, Value};
use crate::messages::{self, FieldType, MessageType};
use crate::ns::{self, Verb};
use crate::wire::{self, Message};
use anyhow::{Context, Result, bail};
use bson::{Bson, Document};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::path::{Path, PathBuf};

pub const CONSOLE: &str = "Environment.Console";
pub const TCP: &str = "Environment.Tcp";
pub const FILES: &str = "Environment.Files";
pub const PACKED: &str = "Packed.Files";

/// The import path for the runtime's message types. These types are always
/// in scope in an image; importing them is optional.
pub const ENV_MODULE: &str = "othismo/env";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// Every message type the image knows: the runtime's plus its imports.
    pub messages: BTreeMap<String, MessageType>,
    /// Values the host provides, in image order; [`Arg::Slot`] refers to them by index.
    pub slots: Vec<Slot>,
    /// Instances in image order; everything else refers to them by index.
    pub instances: Vec<Instance>,
    /// The files the image was checked from: the `.img` and its imports.
    /// Not kept in a built image, which has copies under its sources.
    #[serde(skip)]
    pub sources: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub kind: Kind,
    /// An environment object's config.
    pub config: Option<Arg>,
    /// The initial view.
    pub mounts: Vec<MountDecl>,
    /// Requests the image makes of this instance at startup.
    pub startup: Vec<Startup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MountDecl {
    /// Clean, absolute.
    pub at: String,
    pub target: usize,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Startup {
    pub verb: Verb,
    /// Clean, in the instance's served tree.
    pub path: String,
    pub body: Option<Arg>,
}

/// A message the image gives, or a slot standing for one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Arg {
    Message(Message),
    Slot(usize),
}

impl Arg {
    /// The message, once any slot has been bound (see [`crate::slots::bind`]).
    pub fn message(&self) -> Result<&Message> {
        match self {
            Arg::Message(m) => Ok(m),
            Arg::Slot(i) => bail!("slot {i} has no value; the image hasn't been bound"),
        }
    }
}

/// `require <name> : <Type> { defaults }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    pub type_name: String,
    /// The image's defaults: some, all or none of the type's fields.
    pub defaults: Document,
    /// Once bound: the value, and where each field came from, in field order.
    #[serde(skip)]
    pub value: Option<Message>,
    #[serde(skip)]
    pub origins: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    /// `instance x : Name` runs `scripts/Name.wasm`.
    Wasm { type_name: String },
    Console,
    /// Listens on the address in its `Listen` config.
    Tcp,
    /// Serves files under the path in its `FilesRoot` config. A relative
    /// path is relative to the image.
    Files,
    /// Serves the files its includes packed into the image, in order.
    Packed { includes: Vec<IncludeDecl> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncludeDecl {
    /// As written, relative to the image.
    pub source: String,
    /// Clean and absolute, without a trailing `/`.
    pub at: Option<String>,
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
    let mut program = check(&img, dir).with_context(|| format!("checking {}", path.display()))?;
    program.sources.insert(0, path.to_path_buf());
    Ok(program)
}

/// The runtime's own message types, from its header (`as/env.iface`).
pub fn env_messages() -> BTreeMap<String, MessageType> {
    crate::iface::env().messages.clone()
}

/// Validates `img`; relative import paths resolve against `dir`.
pub fn check(img: &Image, dir: &Path) -> Result<Program> {
    let (messages, sources) = resolve_imports(img, dir)?;

    let mut slots: Vec<Slot> = vec![];
    for req in &img.requires {
        let name = &req.name.name;
        if messages.contains_key(name) {
            bail!(err_at(&req.name, format!("`{name}` is a message type; a slot needs a name of its own")));
        }
        if slots.iter().any(|s| s.name == *name) {
            bail!(err_at(&req.name, format!("slot `{name}` is declared twice")));
        }
        let (type_name, defaults) = body_fields(&messages, &req.body, false)?;
        slots.push(Slot { name: name.clone(), type_name, defaults, value: None, origins: vec![] });
    }

    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, inst) in img.instances.iter().enumerate() {
        if index.insert(&inst.name.name, i).is_some() {
            bail!(err_at(&inst.name, format!("instance `{}` is declared twice", inst.name.name)));
        }
    }

    let mut instances = vec![];
    for inst in &img.instances {
        let ty = &inst.type_name;
        let mut config = None;
        if ty.name != PACKED
            && let Some(inc) = inst.includes.first()
        {
            bail!("{}: `include` packs files into a {PACKED} instance, and `{}` is a {}", inc.pos, inst.name.name, ty.name);
        }
        let builtin = BUILTINS.iter().find(|(name, _)| *name == ty.name);
        // A built-in object's config is the image's `control` of it, taken
        // before the object is made rather than sent as a request.
        let (controls, requests): (Vec<_>, Vec<_>) = if builtin.is_some() || ty.name == PACKED {
            inst.requests.iter().partition(|r| r.verb == Verb::Control)
        } else {
            (vec![], inst.requests.iter().collect())
        };
        let kind = match builtin {
            Some(&(name, config_type)) => {
                if let Some(m) = inst.mounts.first() {
                    bail!(err_at(&m.target, format!("{name} has no view to mount into; it only serves")));
                }
                if let Some(c) = controls.iter().find(|c| c.path.path != "/") {
                    bail!("{}: {name} is configured at its root, with `control /`", c.path.pos);
                }
                let needs = || {
                    let t = config_type.unwrap();
                    err_at(&inst.name, format!("{name} needs exactly one config message, `control / {t} {{ ... }}`"))
                };
                config = match (config_type, controls.as_slice()) {
                    (None, []) => None,
                    (None, [c, ..]) => bail!("{}: {name} takes no config", c.verb_pos),
                    (Some(t), [c]) => {
                        let arg = body_arg(&messages, &slots, c.body.as_ref().ok_or_else(needs)?)?;
                        if arg_type(&slots, &arg) != t {
                            bail!(needs());
                        }
                        Some(arg)
                    }
                    (Some(_), _) => bail!(needs()),
                };
                builtin_kind(name)
            }
            None if ty.name == PACKED => {
                if let Some(m) = inst.mounts.first() {
                    bail!(err_at(&m.target, format!("{PACKED} has no view to mount into; it only serves")));
                }
                if let Some(c) = controls.first() {
                    bail!("{}: {PACKED} takes no config; `include` the files it serves", c.verb_pos);
                }
                let mut includes = vec![];
                for inc in &inst.includes {
                    let at = match &inc.at {
                        Some(p) => Some(
                            ns::mount_point(&p.path)
                                .map_err(|_| anyhow::anyhow!("{}: `{}` is not a clean absolute path", p.pos, p.path))?,
                        ),
                        None => None,
                    };
                    includes.push(IncludeDecl { source: inc.source.clone(), at });
                }
                Kind::Packed { includes }
            }
            None if ty.name.starts_with("Environment.") || ty.name.starts_with("Packed.") => {
                let known: Vec<&str> = BUILTINS.iter().map(|(n, _)| *n).chain([PACKED]).collect();
                bail!(err_at(ty, format!("unknown built-in type `{}` (there are: {})", ty.name, known.join(", "))));
            }
            None if ty.name.contains('.') => bail!(err_at(ty, format!("`{}` is not a module name", ty.name))),
            None => Kind::Wasm { type_name: ty.name.clone() },
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
        for r in requests {
            let path = ns::clean(&r.path.path)
                .map_err(|_| anyhow::anyhow!("{}: `{}` is not a clean absolute path", r.path.pos, r.path.path))?;
            let body = r.body.as_ref().map(|b| body_arg(&messages, &slots, b)).transpose()?;
            startup.push(Startup { verb: r.verb, path, body });
        }

        instances.push(Instance { name: inst.name.name.clone(), kind, config, mounts, startup });
    }

    Ok(Program { messages, slots, instances, sources })
}

/// A summary of each instance's view and the most it can ever reach, for
/// `--check`. `about` may say more about an instance, on a line of its own.
pub fn describe(p: &Program, about: &dyn Fn(usize) -> Option<String>) -> String {
    let mounts: Vec<Vec<(usize, bool)>> =
        p.instances.iter().map(|i| i.mounts.iter().map(|m| (m.target, m.read_only)).collect()).collect();
    let reach = ns::reach(&mounts);
    let width = p.instances.iter().map(|i| i.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (i, inst) in p.instances.iter().enumerate() {
        let mut name = inst.name.as_str();
        if let Some(about) = about(i) {
            writeln!(out, "{name:width$}  {about}").unwrap();
            name = "";
        }
        if inst.mounts.is_empty() {
            if !name.is_empty() {
                writeln!(out, "{name:width$}  sees nothing").unwrap();
            }
            continue;
        }
        let sees: Vec<String> = inst
            .mounts
            .iter()
            .map(|m| format!("{} → {}{}", m.at, p.instances[m.target].name, if m.read_only { " ro" } else { "" }))
            .collect();
        writeln!(out, "{name:width$}  sees {}", sees.join(", ")).unwrap();
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

fn builtin_kind(name: &str) -> Kind {
    match name {
        CONSOLE => Kind::Console,
        TCP => Kind::Tcp,
        FILES => Kind::Files,
        other => unreachable!("unknown builtin {other}"),
    }
}

/// Every message type in scope, and the files they were read from.
fn resolve_imports(img: &Image, dir: &Path) -> Result<(BTreeMap<String, MessageType>, Vec<PathBuf>)> {
    let mut parsed: HashMap<PathBuf, BTreeMap<String, MessageType>> = HashMap::new();
    let env = env_messages();
    let mut out = env.clone();
    let mut imported: HashMap<String, PathBuf> = HashMap::new();
    let mut sources = vec![];
    for imp in &img.imports {
        let is_env = match &imp.from {
            image::Source::Module(m) if m.name == "env" => true,
            image::Source::Path(p) if p == ENV_MODULE => true,
            _ => false,
        };
        let path = match &imp.from {
            _ if is_env => PathBuf::from(ENV_MODULE),
            image::Source::Path(p) if p.starts_with('.') || p.starts_with('/') => {
                let mut path = dir.join(p);
                if path.extension().is_none() {
                    path.set_extension("ts");
                }
                path
            }
            other => bail!("{}: unknown module {other}; use a relative path or `env`", imp.pos),
        };
        if !parsed.contains_key(&path) {
            let decls = if is_env {
                env.clone()
            } else {
                let src = std::fs::read_to_string(&path)
                    .with_context(|| format!("{}: cannot read import {}", imp.pos, path.display()))?;
                sources.push(path.clone());
                messages::parse(&src).with_context(|| format!("in {}", path.display()))?
            };
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
    Ok((out, sources))
}

/// A message literal, or a slot's name standing alone.
fn body_arg(messages: &BTreeMap<String, MessageType>, slots: &[Slot], body: &image::Body) -> Result<Arg> {
    let tid = &body.type_name;
    if let Some(i) = slots.iter().position(|s| s.name == tid.name) {
        if body.fields.is_some() {
            bail!(err_at(tid, format!("`{}` is a slot; its fields come from the host, so use it alone", tid.name)));
        }
        return Ok(Arg::Slot(i));
    }
    let (type_name, fields) = body_fields(messages, body, true)?;
    Ok(Arg::Message(Message { type_name, fields }))
}

fn arg_type<'a>(slots: &'a [Slot], arg: &'a Arg) -> &'a str {
    match arg {
        Arg::Message(m) => &m.type_name,
        Arg::Slot(i) => &slots[*i].type_name,
    }
}

/// A message literal's type and fields. With `complete`, every field must be given.
fn body_fields(messages: &BTreeMap<String, MessageType>, body: &image::Body, complete: bool) -> Result<(String, Document)> {
    let tid = &body.type_name;
    let Some(ty) = messages.get(&tid.name) else {
        bail!(err_at(tid, format!("unknown message type `{}`; is it imported?", tid.name)));
    };
    let given = body.fields.as_deref().unwrap_or(&[]);
    let mut fields = Document::new();
    for (fname, value) in given {
        if fields.contains_key(&fname.name) {
            bail!(err_at(fname, format!("field `{}` given twice", fname.name)));
        }
        fields.insert(fname.name.clone(), field_bson(ty, &fname.name, value).map_err(|e| err_at(fname, e))?);
    }
    if complete && let Some((missing, _)) = ty.fields.iter().find(|(n, _)| !fields.contains_key(n)) {
        bail!(err_at(tid, format!("`{}` is missing field `{missing}`", tid.name)));
    }
    Ok((tid.name.clone(), fields))
}

/// The BSON for `value` as field `field` of `ty`, if it fits the field's type.
pub fn field_bson(ty: &MessageType, field: &str, value: &Value) -> std::result::Result<Bson, String> {
    let Some(fty) = ty.field(field) else {
        return Err(format!("`{}` has no field `{field}`", ty.name));
    };
    Ok(match (fty, value) {
        (FieldType::String, Value::Str(s)) => Bson::String(s.clone()),
        (FieldType::Bool, Value::Bool(b)) => Bson::Boolean(*b),
        (FieldType::F32 | FieldType::F64, Value::Float(x)) => Bson::Double(*x),
        (FieldType::F32 | FieldType::F64, Value::Int(i)) => Bson::Double(*i as f64),
        (FieldType::Bytes, Value::Str(s)) => wire::bytes(s.as_bytes().to_vec()),
        (t, Value::Int(i)) if t.int_range().is_some() => {
            let (lo, hi) = t.int_range().unwrap();
            if !(lo..=hi).contains(i) {
                return Err(format!("{i} does not fit in {t}"));
            }
            wire::int_to_bson(t, *i)
        }
        (t, v) => return Err(format!("`{}.{field}` is {t}, but was given {v:?}", ty.name)),
    })
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
        let dir = std::env::temp_dir().join(format!("othismo_check_{}_{:?}", std::process::id(), std::thread::current().id()));
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
        assert_eq!(p.instances[1].kind, Kind::Wasm { type_name: "Echo".into() });
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
        let body = s[0].body.as_ref().unwrap().message().unwrap();
        assert_eq!(body.fields.get_i32("count").unwrap(), 3);
        assert_eq!(body.fields.get_f64("ratio").unwrap(), 2.0);
        assert_eq!(s[1].body.as_ref().unwrap().message().unwrap().type_name, "Ping");
        assert_eq!((s[2].verb, s[2].path.as_str(), &s[2].body), (Verb::Remove, "/a/b/", &None));

        assert!(err(&inst("write /c Config { name: \"x\" }")).contains("missing field `count`"));
        assert!(err(&inst("write /c Config { name: 1, count: 1, ratio: 1, on: true }")).contains("is string"));
        assert!(err(&inst("write /c Config { name: \"\", count: 256, ratio: 1, on: true }")).contains("does not fit in u8"));
        assert!(err(&inst("invoke / Ping { x: 1 }")).contains("no field `x`"));
        assert!(err(&inst("invoke / Missing")).contains("unknown message type"));
        assert!(err(&inst("write / Blob { data: 1 }")).contains("is ArrayBuffer"));
        assert!(err(&inst("read /../x")).contains("not a clean absolute path"));
    }

    #[test]
    fn bytes_fields_take_string_literals_as_utf8() {
        let p = check_src(&format!("{HEADER} instance e : Echo {{ write / Blob {{ data: \"hé\" }} }}")).unwrap();
        let body = p.instances[1].startup[0].body.as_ref().unwrap().message().unwrap();
        let data = body.fields.get_binary_generic("data").unwrap();
        assert_eq!(data, "hé".as_bytes());
    }

    #[test]
    fn environment_objects_take_config_from_the_image() {
        let p = check_src(
            r#"instance net : Environment.Tcp { control / Listen { address: "127.0.0.1:0" } }
            instance fs : Environment.Files { control / FilesRoot { path: "public" } }"#,
        )
        .unwrap();
        assert_eq!(p.instances[0].kind, Kind::Tcp);
        let config = p.instances[0].config.as_ref().unwrap().message().unwrap();
        assert_eq!(config.fields.get_str("address").unwrap(), "127.0.0.1:0");
        assert_eq!(p.instances[1].kind, Kind::Files);
        assert!(p.instances[0].startup.is_empty(), "config isn't a request");

        let e = err("instance net : Environment.Tcp");
        assert!(e.contains("needs exactly one config message, `control / Listen"), "{e}");
        let e = err(r#"instance net : Environment.Tcp { control / Listen { address: "a" } control / Listen { address: "b" } }"#);
        assert!(e.contains("exactly one"), "{e}");
        let e = err(r#"instance c : Environment.Console { control / Listen { address: "a" } }"#);
        assert!(e.contains("takes no config"), "{e}");
        let e = err(r#"instance net : Environment.Tcp { control /x Listen { address: "a" } }"#);
        assert!(e.contains("configured at its root"), "{e}");
        let e = err("instance c : Environment.Stdin");
        assert!(e.contains("unknown built-in type `Environment.Stdin`"), "{e}");
    }

    #[test]
    fn packed_files_take_includes_and_nothing_else() {
        let p = check_src(
            r#"instance files : Packed.Files { include "public/" include "a.txt" at /b//c/ }
            instance site : Site { mount files /files }"#,
        )
        .unwrap();
        let want = [
            IncludeDecl { source: "public/".into(), at: None },
            IncludeDecl { source: "a.txt".into(), at: Some("/b/c".into()) },
        ];
        assert_eq!(p.instances[0].kind, Kind::Packed { includes: want.to_vec() });

        let e = err(r#"instance s : Site { include "x" }"#);
        assert!(e.contains("`include` packs files into a Packed.Files instance, and `s` is a Site"), "{e}");
        let e = err(r#"instance f : Packed.Files { include "x" at /../y }"#);
        assert!(e.contains("not a clean absolute path"), "{e}");
        let e = err(r#"instance f : Packed.Files { control / FilesRoot { path: "." } }"#);
        assert!(e.contains("takes no config"), "{e}");
        let e = err(r#"instance a : A instance f : Packed.Files { mount a /a }"#);
        assert!(e.contains("only serves"), "{e}");
        let e = err(r#"instance f : Packed.Dirs"#);
        assert!(e.contains("unknown built-in type `Packed.Dirs` (there are: Environment.Console, Environment.Tcp, Environment.Files, Packed.Files)"), "{e}");
    }

    #[test]
    fn slots_stand_in_for_messages() {
        let p = check_src(&format!(
            r#"{HEADER}
            require listen : Listen
            require config : Config {{ name: "x", count: 3 }}
            instance net : Environment.Tcp {{ control / listen }}
            instance e : Echo {{ write /config config }}"#
        ))
        .unwrap();
        assert_eq!(p.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["listen", "config"]);
        assert_eq!(p.slots[1].defaults, bson::doc! { "name": "x", "count": 3 }, "defaults may be partial");
        assert_eq!(p.instances[1].config, Some(Arg::Slot(0)));
        assert_eq!(p.instances[2].startup[0].body, Some(Arg::Slot(1)));

        let inst = |decls: &str, member: &str| format!("{HEADER} {decls} instance e : Echo {{ {member} }}");
        let e = err(&inst("require Greeting : Greeting", ""));
        assert!(e.contains("`Greeting` is a message type; a slot needs a name of its own"), "{e}");
        let e = err(&inst("require g : Greeting require g : Ping", ""));
        assert!(e.contains("slot `g` is declared twice"), "{e}");
        let e = err(&inst("require g : Greeting { text: 1 }", ""));
        assert!(e.contains("`Greeting.text` is string"), "{e}");
        let e = err(&inst("require g : Greeting { nope: 1 }", ""));
        assert!(e.contains("`Greeting` has no field `nope`"), "{e}");
        let e = err(&inst("require g : Nope", ""));
        assert!(e.contains("unknown message type `Nope`"), "{e}");
        let e = err(&inst("require g : Greeting", "write / g { text: \"x\" }"));
        assert!(e.contains("`g` is a slot; its fields come from the host, so use it alone"), "{e}");
        let e = err("require t : Text instance net : Environment.Tcp { control / t }");
        assert!(e.contains("needs exactly one config message, `control / Listen"), "{e}");
    }

    #[test]
    fn runtime_types_can_only_be_redeclared_exactly() {
        let dir = std::env::temp_dir().join(format!("othismo_envshape_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.ts"), "export class Listen { port: i32 }\nexport class Line { text: string }").unwrap();
        let ok = image::parse(r#"import { Line } from "./m" import { Text } from "othismo/env""#).unwrap();
        assert!(check(&ok, &dir).is_ok());
        let bad = image::parse(r#"import { Listen } from "./m""#).unwrap();
        let e = format!("{:#}", check(&bad, &dir).unwrap_err());
        let missing = image::parse(r#"import { Nope } from "othismo/env""#).unwrap();
        let e2 = format!("{:#}", check(&missing, &dir).unwrap_err());
        std::fs::remove_dir_all(&dir).ok();
        assert!(e.contains("`Listen` is a runtime type and must be declared as { address: string }"), "{e}");
        assert!(e2.contains("not an exported class"), "{e2}");
    }

    #[test]
    fn describes_views_and_reach() {
        let p = check_src(
            r#"instance cons : Environment.Console
            instance files : Environment.Files { control / FilesRoot { path: "." } }
            instance site : Site { mount files /files readonly }
            instance http : Http { mount site /www  mount cons /dev/cons }"#,
        )
        .unwrap();
        assert_eq!(
            describe(&p, &|_| None),
            "cons   sees nothing\n\
             files  sees nothing\n\
             site   sees /files → files ro\n       can reach at most: files ro\n\
             http   sees /www → site, /dev/cons → cons\n       can reach at most: cons rw, files ro (via site), site rw\n"
        );
    }
}
