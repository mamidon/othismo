//! Resolves and validates a parsed image into a [`Program`] the runtime can run.

use crate::iface::{self, Header, Object};
use crate::image::{self, Ident, Image, Source, Value};
use crate::messages::{FieldType, MessageType};
use crate::ns::{self, Namespace, Status, Verb};
use crate::pattern::Pattern;
use crate::wire::{self, Message};
use anyhow::{Context, Result, anyhow, bail};
use bson::{Bson, Document};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::path::{Path, PathBuf};

pub const CONSOLE: &str = "Environment.Console";
pub const TCP: &str = "Environment.Tcp";
pub const FILES: &str = "Environment.Files";
pub const PACKED: &str = "Packed.Files";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// Every message type the image knows: the runtime's, every module's
    /// and its imports'.
    pub messages: BTreeMap<String, MessageType>,
    /// Each instance type's interface, from its module's header or, for a
    /// built-in type, from the runtime's.
    pub interfaces: BTreeMap<String, Object>,
    /// Values the host provides, in image order; [`Arg::Slot`] refers to them by index.
    pub slots: Vec<Slot>,
    /// Instances in image order; everything else refers to them by index.
    pub instances: Vec<Instance>,
    /// The image's own view, which its startup requests resolve in.
    pub view: Vec<MountDecl>,
    /// The image's startup requests, in the order it makes them.
    pub startup: Vec<Startup>,
    /// The files the image was checked from: the `.img` and the headers it
    /// imports from files.
    /// Not kept in a built image, which has copies under its sources.
    #[serde(skip)]
    pub sources: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    /// As written: a module name, or a built-in type such as `Environment.Tcp`.
    pub type_name: String,
    pub kind: Kind,
    /// An environment object's config.
    pub config: Option<Arg>,
    /// The initial view.
    pub mounts: Vec<MountDecl>,
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
    /// Clean, in the image's view.
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

/// Reads, parses and checks the image at `path`, reading each module's
/// header from the module.
pub fn load(path: &Path) -> Result<Program> {
    let src = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let img = image::parse(&src).with_context(|| format!("parsing {}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut modules = |name: &str| module_header(dir, name);
    let mut program = check(&img, dir, &mut modules).with_context(|| format!("checking {}", path.display()))?;
    program.sources.insert(0, path.to_path_buf());
    Ok(program)
}

/// Module `name`'s header, from the module itself: `scripts/<name>.wasm`
/// next to the image in `dir`.
pub fn module_header(dir: &Path, name: &str) -> Result<Header> {
    let path = crate::pack::module_path(dir, name);
    let wasm = std::fs::read(&path).with_context(|| format!("cannot read {} (did you compile {name}.ts?)", path.display()))?;
    let header = iface::embedded(&wasm).with_context(|| path.display().to_string())?.ok_or_else(|| {
        anyhow!("{} has no header; embed scripts/{name}.iface into it with `othismo embed`", path.display())
    })?;
    iface::for_module(&header, name).with_context(|| format!("{} has the wrong header", path.display()))?;
    Ok(header)
}

/// The runtime's own message types, from its header (`as/env.iface`).
pub fn env_messages() -> BTreeMap<String, MessageType> {
    iface::env().messages.clone()
}

/// Validates `img`: its messages against their types, and its wiring
/// against its objects' interfaces. `modules` gives each module's header,
/// and imports from files resolve against `dir`.
pub fn check(img: &Image, dir: &Path, modules: &mut dyn FnMut(&str) -> Result<Header>) -> Result<Program> {
    let mut headers: BTreeMap<String, Header> = BTreeMap::new();
    for inst in &img.instances {
        let ty = &inst.type_name;
        if !ty.name.contains('.') && !headers.contains_key(&ty.name) {
            let header = modules(&ty.name).with_context(|| format!("{}: instance `{}`", ty.pos, inst.name.name))?;
            headers.insert(ty.name.clone(), header);
        }
    }
    // Only the runtime's types and the image's imports can be written in it.
    let (messages_in_scope, sources) = resolve_imports(img, dir, &mut headers, modules)?;
    let messages = all_messages(&headers, &messages_in_scope)?;
    let mut interfaces: BTreeMap<String, Object> =
        headers.iter().filter_map(|(name, h)| Some((name.clone(), h.object(name)?.clone()))).collect();
    for inst in &img.instances {
        if let Some(o) = iface::env().object(&inst.type_name.name) {
            interfaces.insert(o.name.clone(), o.clone());
        }
    }

    let mut slots: Vec<Slot> = vec![];
    for req in &img.requires {
        let name = &req.name.name;
        if messages_in_scope.contains_key(name) {
            bail!(err_at(&req.name, format!("`{name}` is a message type; a slot needs a name of its own")));
        }
        if slots.iter().any(|s| s.name == *name) {
            bail!(err_at(&req.name, format!("slot `{name}` is declared twice")));
        }
        let (type_name, defaults) = body_fields(&messages_in_scope, &req.body, false)?;
        slots.push(Slot { name: name.clone(), type_name, defaults, value: None, origins: vec![] });
    }

    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, inst) in img.instances.iter().enumerate() {
        if index.insert(&inst.name.name, i).is_some() {
            bail!(err_at(&inst.name, format!("instance `{}` is declared twice", inst.name.name)));
        }
    }

    // The image's requests resolve in its own view, as an instance's do in
    // its. Nothing has exposed anything yet, so this is where they land.
    let view = mount_decls(&img.mounts, &index, "the image's view")?;
    let image_view = img.instances.len();
    let mut image_ns = Namespace::new(image_view + 1);
    for m in &view {
        image_ns.mount(image_view, &m.at, m.target, m.read_only);
    }
    // (the instance it lands in, the path there, the path in the image's view)
    let mut addressed: Vec<(usize, String, String)> = vec![];
    for r in &img.requests {
        let full =
            ns::clean(&r.path.path).map_err(|_| anyhow!("{}: `{}` is not a clean absolute path", r.path.pos, r.path.path))?;
        let to = image_ns
            .resolve(image_view, &full)
            .map_err(|_| anyhow!("{}: nothing is mounted at `{full}` in the image's view", r.path.pos))?;
        if to.read_only && !r.verb.is_safe() {
            bail!("{}: `{full}` is read-only in the image's view, so it can't take `{}`", r.verb_pos, r.verb);
        }
        addressed.push((to.inst, to.path, full));
    }
    let is_config = |to: usize, verb: Verb| {
        let ty = img.instances[to].type_name.name.as_str();
        verb == Verb::Control && (ty == PACKED || BUILTINS.iter().any(|(name, _)| *name == ty))
    };

    let mut instances = vec![];
    for (i, inst) in img.instances.iter().enumerate() {
        let ty = &inst.type_name;
        let requests: Vec<(&image::Request, &str)> =
            img.requests.iter().zip(&addressed).filter(|(_, (to, ..))| *to == i).map(|(r, (_, p, _))| (r, p.as_str())).collect();
        let mut config = None;
        let obj = interfaces.get(&ty.name);
        if ty.name != PACKED
            && let Some(inc) = inst.includes.first()
        {
            bail!("{}: `include` packs files into a {PACKED} instance, and `{}` is a {}", inc.pos, inst.name.name, ty.name);
        }
        let builtin = BUILTINS.iter().find(|(name, _)| *name == ty.name);
        // A built-in object's config is the image's `control` of it, taken
        // before the object is made rather than sent as a request.
        let controls: Vec<&(&image::Request, &str)> = requests.iter().filter(|(r, _)| is_config(i, r.verb)).collect();
        let kind = match builtin {
            Some(&(name, config_type)) => {
                if let Some(m) = inst.mounts.first() {
                    bail!(err_at(&m.target, format!("{name} has no view to mount into; it only serves")));
                }
                if let Some((c, path)) = controls.iter().find(|(_, path)| *path != "/") {
                    bail!("{}: {name} is configured at its root, and `{}` is `{path}` in it", c.path.pos, c.path.path);
                }
                let needs = || {
                    let t = config_type.unwrap();
                    err_at(
                        &inst.name,
                        format!("{name} needs exactly one config message: mount it in the image's view and `control` its root with a {t}"),
                    )
                };
                config = match (config_type, controls.as_slice()) {
                    (None, []) => None,
                    (None, [(c, _), ..]) => bail!("{}: {name} takes no config", c.verb_pos),
                    (Some(t), [(c, _)]) => {
                        let arg = body_arg(&messages_in_scope, &slots, c.body.as_ref().ok_or_else(needs)?)?;
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
                if let Some((c, _)) = controls.first() {
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

        let mounts = mount_decls(&inst.mounts, &index, &format!("`{}`'s view", inst.name.name))?;

        if let Some(obj) = obj {
            check_requests(inst, obj, &requests, &messages_in_scope, &slots)?;
            check_mounts(img, inst, obj, &index, &interfaces)?;
        }
        instances.push(Instance { name: inst.name.name.clone(), type_name: ty.name.clone(), kind, config, mounts });
    }

    let mut startup = vec![];
    for (r, (to, _, full)) in img.requests.iter().zip(&addressed) {
        if is_config(*to, r.verb) {
            continue;
        }
        let body = r.body.as_ref().map(|b| body_arg(&messages_in_scope, &slots, b)).transpose()?;
        startup.push(Startup { verb: r.verb, path: full.clone(), body });
    }

    Ok(Program { messages, interfaces, slots, instances, view, startup, sources })
}

/// A view's mounts, resolved against the instances in `index`. `whose`
/// names the view in errors.
fn mount_decls(mounts: &[image::Mount], index: &HashMap<&str, usize>, whose: &str) -> Result<Vec<MountDecl>> {
    let mut out: Vec<MountDecl> = vec![];
    for m in mounts {
        let target =
            *index.get(m.target.name.as_str()).ok_or_else(|| err_at(&m.target, format!("unknown instance `{}`", m.target.name)))?;
        let at = ns::mount_point(&m.at.path).map_err(|_| anyhow!("{}: `{}` is not a clean absolute path", m.at.pos, m.at.path))?;
        if out.iter().any(|prev| prev.at == at) {
            bail!("{}: `{at}` is already mounted in {whose}", m.at.pos);
        }
        out.push(MountDecl { at, target, read_only: m.read_only });
    }
    Ok(out)
}

/// Each of the image's requests of `inst`, with its path in `inst`'s
/// served tree, against what `obj` serves.
fn check_requests(
    inst: &image::Instance,
    obj: &Object,
    requests: &[(&image::Request, &str)],
    scope: &BTreeMap<String, MessageType>,
    slots: &[Slot],
) -> Result<()> {
    let name = &inst.name.name;
    for &(r, path) in requests {
        let body = r.body.as_ref().map(|b| body_arg(scope, slots, b)).transpose()?;
        let body_type = body.as_ref().map(|b| arg_type(slots, b));
        let Some(entry) = obj.serving(path) else {
            bail!("{}: `{name}` serves nothing at `{path}`", r.verb_pos);
        };
        let given = match body_type {
            Some(t) => format!("{} {t}", r.verb),
            None => r.verb.to_string(),
        };
        match entry.takes(r.verb, body_type) {
            Ok(_) => {}
            Err(Status::NotSupported) => bail!("{}: `{name}` doesn't take `{}` at `{path}`; it serves `{entry}`", r.verb_pos, r.verb),
            Err(_) => bail!("{}: `{name}` doesn't take `{given}` at `{path}`; it serves `{entry}`", r.verb_pos),
        }
    }
    Ok(())
}

/// Each of `inst`'s mounts against what `obj` needs there, and each need
/// for one mount point in particular against the mounts.
fn check_mounts(
    img: &Image,
    inst: &image::Instance,
    obj: &Object,
    index: &HashMap<&str, usize>,
    interfaces: &BTreeMap<String, Object>,
) -> Result<()> {
    let name = &inst.name.name;
    let mut ats = vec![];
    for m in &inst.mounts {
        let at = ns::mount_point(&m.at.path).map_err(|_| anyhow!("{}: `{}` is not a clean absolute path", m.at.pos, m.at.path))?;
        ats.push(at.clone());
        let Some(need) = obj.need(&at) else { continue };
        let target = &img.instances[index[m.target.name.as_str()]];
        let Some(served) = interfaces.get(&target.type_name.name) else { continue };
        for e in &need.entries {
            for r in &e.requests {
                if m.read_only && !r.verb.is_safe() {
                    bail!("{}: `{name}` needs `{} {r}` at `{at}`, but `{}` is mounted there read-only", m.at.pos, e.path, m.target.name);
                }
                if !serves_need(served, need, e, r) {
                    let shown: Vec<String> =
                        served.serves.iter().filter(|s| s.path.overlaps(&e.path)).map(|s| format!("`{s}`")).collect();
                    let has = if shown.is_empty() { "nothing there".to_string() } else { shown.join(", ") };
                    bail!(
                        "{}: `{name}` needs `{} {r}` at `{at}`, and `{}` serves {has}",
                        m.at.pos,
                        e.path,
                        m.target.name
                    );
                }
            }
        }
    }
    for need in obj.needs.iter().filter(|n| n.at.is_literal()) {
        if !ats.iter().any(|at| need.at.matches(at)) {
            bail!("{}: `{name}` needs something mounted at `{}`", inst.name.pos, need.at);
        }
    }
    Ok(())
}

/// Whether `served` meets request `r` of entry `want` in block `need`: an
/// entry covering every path `want` names takes it, and no more specific
/// entry in between takes its verb in a way that doesn't.
fn serves_need(served: &Object, need: &iface::Need, want: &iface::Entry, r: &iface::Request) -> bool {
    let takes = |e: &iface::Entry| e.requests.iter().any(|s| s.meets(r));
    let covering = served.serves.iter().filter(|e| e.path.covers(&want.path) && takes(e));
    let Some(cover) = covering.max_by(|a, b| a.path.specificity(&b.path)) else { return false };
    // Paths a more specific entry of the need's own block covers are that entry's concern.
    let narrower = |p: &Pattern| {
        need.entries.iter().any(|n| n.path.specificity(&want.path) == Ordering::Greater && n.path.covers(p))
    };
    served
        .serves
        .iter()
        .filter(|e| e.path.overlaps(&want.path) && e.path.specificity(&cover.path) == Ordering::Greater)
        .filter(|e| !narrower(&e.path))
        .all(|e| takes(e) || !e.requests.iter().any(|s| s.verb == r.verb))
}

/// A summary of each instance's view and the most it can ever reach, for
/// `--check`. `about` may say more about an instance, on a line of its own.
pub fn describe(p: &Program, about: &dyn Fn(usize) -> Option<String>) -> String {
    let mounts: Vec<Vec<(usize, bool)>> =
        p.instances.iter().map(|i| i.mounts.iter().map(|m| (m.target, m.read_only)).collect()).collect();
    let reach = ns::reach(&mounts);
    let width = p.instances.iter().map(|i| i.name.len()).max().unwrap_or(0);
    let sees = |mounts: &[MountDecl]| -> String {
        let sees: Vec<String> = mounts
            .iter()
            .map(|m| format!("{} → {}{}", m.at, p.instances[m.target].name, if m.read_only { " ro" } else { "" }))
            .collect();
        sees.join(", ")
    };
    let mut out = String::new();
    if !p.view.is_empty() {
        writeln!(out, "the image sees {}\n", sees(&p.view)).unwrap();
    }
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
        writeln!(out, "{name:width$}  sees {}", sees(&inst.mounts)).unwrap();
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

/// The message types an image can write: the runtime's and its imports,
/// and the header files it imported them from.
fn resolve_imports(
    img: &Image,
    dir: &Path,
    headers: &mut BTreeMap<String, Header>,
    modules: &mut dyn FnMut(&str) -> Result<Header>,
) -> Result<(BTreeMap<String, MessageType>, Vec<PathBuf>)> {
    let env = env_messages();
    let mut out = env.clone();
    let mut imported: HashMap<String, String> = HashMap::new();
    let mut files: HashMap<PathBuf, Header> = HashMap::new();
    let mut sources = vec![];
    for imp in &img.imports {
        let header = match &imp.from {
            Source::Module(m) if m.name == "env" => iface::env(),
            Source::Module(m) => {
                if !headers.contains_key(&m.name) {
                    let h = modules(&m.name).with_context(|| format!("{}: importing from {}", imp.pos, m.name))?;
                    headers.insert(m.name.clone(), h);
                }
                &headers[&m.name]
            }
            Source::Path(p) if p == "othismo/env" => {
                bail!("{}: the runtime's types are imported `from env`", imp.pos)
            }
            Source::Path(p) if p.ends_with(".ts") || !(p.starts_with('.') || p.starts_with('/')) => bail!(
                "{}: types are imported from headers: `from env`, `from <Module>` or `from \"./<file>.iface\"`",
                imp.pos
            ),
            Source::Path(p) => {
                let mut path = dir.join(p);
                if path.extension().is_none() {
                    path.set_extension("iface");
                }
                if !files.contains_key(&path) {
                    let h = iface::load(&path).with_context(|| format!("{}: importing from {p:?}", imp.pos))?;
                    sources.push(path.clone());
                    files.insert(path.clone(), h);
                }
                &files[&path]
            }
        };
        for name in &imp.names {
            let Some(ty) = header.messages.get(&name.name) else {
                bail!(err_at(name, format!("{} has no message type `{}`", imp.from, name.name)));
            };
            if imported.insert(name.name.clone(), imp.from.to_string()).is_some() {
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

/// Every message type in the image: the runtime's, every module's and the
/// imported ones. A type is its name and fields, so two declarations of
/// one name must agree.
fn all_messages(headers: &BTreeMap<String, Header>, scope: &BTreeMap<String, MessageType>) -> Result<BTreeMap<String, MessageType>> {
    let mut out: BTreeMap<String, MessageType> = BTreeMap::new();
    let mut from: BTreeMap<String, String> = BTreeMap::new();
    let sets = std::iter::once(("the runtime", &iface::env().messages))
        .chain(headers.iter().map(|(name, h)| (name.as_str(), &h.messages)))
        .chain(std::iter::once(("the image's imports", scope)));
    for (origin, types) in sets {
        for (name, ty) in types {
            match out.get(name) {
                Some(prev) if prev.fields != ty.fields => bail!(
                    "`{name}` is declared one way by {} and another by {origin}; an image can't use both",
                    from[name]
                ),
                Some(_) => {}
                None => {
                    out.insert(name.clone(), ty.clone());
                    from.insert(name.clone(), origin.to_string());
                }
            }
        }
    }
    Ok(out)
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
        message Greeting { text: string }
        message Config { name: string, count: u8, ratio: f64, on: bool }
        message Ping {}
        message Blob { data: bytes }
    "#;

    /// A resolved header, whose imports can only be from env.
    fn header(src: &str) -> Header {
        let file = iface::parse(src).unwrap();
        iface::resolve(&file, &mut |_| Ok(iface::env().clone()), &|_| unreachable!()).unwrap()
    }

    /// A module that needs nothing and serves anything, anywhere.
    fn permissive(name: &str) -> Header {
        let reqs: Vec<String> = ns::VERBS.iter().flat_map(|v| [format!("{v} -> Opaque*"), format!("{v} Opaque -> Opaque*")]).collect();
        header(&format!("object {name} {{ serves {{ /** {} }} }}", reqs.join(", ")))
    }

    /// Checks `img` next to MESSAGES, in messages.iface. Modules have the
    /// headers in `modules`, or else a permissive one.
    fn check_with(img: &str, modules: &[(&str, &str)]) -> Result<Program> {
        let dir = std::env::temp_dir().join(format!("othismo_check_{}_{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("messages.iface"), MESSAGES).unwrap();
        let mut headers = |name: &str| -> Result<Header> {
            match modules.iter().find(|(n, _)| *n == name) {
                Some((_, src)) => Ok(header(src)),
                None if name == "Ghost" => bail!("no module Ghost"),
                None => Ok(permissive(name)),
            }
        };
        let res = image::parse(img).and_then(|img| check(&img, &dir, &mut headers));
        std::fs::remove_dir_all(&dir).ok();
        res
    }

    fn check_src(img: &str) -> Result<Program> {
        check_with(img, &[])
    }

    fn err(img: &str) -> String {
        format!("{:#}", check_src(img).unwrap_err())
    }

    fn err_with(img: &str, modules: &[(&str, &str)]) -> String {
        format!("{:#}", check_with(img, modules).unwrap_err())
    }

    const HEADER: &str = r#"
        import { Greeting, Config, Ping, Blob } from "./messages.iface"
        instance cons : Environment.Console
    "#;

    #[test]
    fn accepts_hello_world() {
        let p = check_src(&format!("{HEADER} instance echo : Echo {{ mount /dev/cons cons }}")).unwrap();
        assert_eq!(p.instances.len(), 2);
        assert_eq!(p.instances[0].kind, Kind::Console);
        assert_eq!(p.instances[1].kind, Kind::Wasm { type_name: "Echo".into() });
        assert_eq!(p.instances[1].mounts, [MountDecl { at: "/dev/cons".into(), target: 0, read_only: false }]);
        assert!(p.messages.contains_key("Line"), "runtime types are always in scope");
        assert!(p.messages.contains_key("Greeting"));
    }

    #[test]
    fn mounts_are_checked() {
        assert!(err(&format!("{HEADER} instance a : A {{ mount /g ghost }}")).contains("unknown instance `ghost`"));
        let e = err(&format!("{HEADER} instance a : A {{ mount /x/../y cons }}"));
        assert!(e.contains("`/x/../y` is not a clean absolute path"), "{e}");
        let e = err(&format!("{HEADER} instance a : A {{ mount /c cons mount /c/ cons }}"));
        assert!(e.contains("`/c` is already mounted"), "{e}");
        let e = err(&format!("{HEADER} instance net : Environment.Console {{ mount /c cons }}"));
        assert!(e.contains("only serves"), "{e}");
        // Forward references are fine: views are built once every instance exists.
        let p = check_src(&format!("{HEADER} instance a : A {{ mount /b b readonly }} instance b : B")).unwrap();
        assert_eq!(p.instances[1].mounts[0], MountDecl { at: "/b".into(), target: 2, read_only: true });
    }

    #[test]
    fn startup_requests_are_typed() {
        let inst = |req: &str| format!("{HEADER} instance e : Echo mount /e e {req}");
        let p = check_src(&inst(
            "write /e/config Config { name: \"x\", count: 3, ratio: 2, on: true } invoke /e Ping remove /e/a//b/",
        ))
        .unwrap();
        let s = &p.startup;
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].verb, s[0].path.as_str()), (Verb::Write, "/e/config"));
        let body = s[0].body.as_ref().unwrap().message().unwrap();
        assert_eq!(body.fields.get_i32("count").unwrap(), 3);
        assert_eq!(body.fields.get_f64("ratio").unwrap(), 2.0);
        assert_eq!(s[1].body.as_ref().unwrap().message().unwrap().type_name, "Ping");
        assert_eq!((s[2].verb, s[2].path.as_str(), &s[2].body), (Verb::Remove, "/e/a/b/", &None));

        assert!(err(&inst("write /e/c Config { name: \"x\" }")).contains("missing field `count`"));
        assert!(err(&inst("write /e/c Config { name: 1, count: 1, ratio: 1, on: true }")).contains("is string"));
        assert!(err(&inst("write /e/c Config { name: \"\", count: 256, ratio: 1, on: true }")).contains("does not fit in u8"));
        assert!(err(&inst("invoke /e Ping { x: 1 }")).contains("no field `x`"));
        assert!(err(&inst("invoke /e Missing")).contains("unknown message type"));
        assert!(err(&inst("write /e Blob { data: 1 }")).contains("is bytes"));
        assert!(err(&inst("read /e/../x")).contains("not a clean absolute path"));
        assert!(err(&inst("read /x")).contains("nothing is mounted at `/x` in the image's view"));
        assert!(err(&inst("read /")).contains("nothing is mounted at `/` in the image's view"));
        assert!(err(&format!("{HEADER} write /e Ping instance e : Echo")).contains("nothing is mounted at `/e`"));
    }

    #[test]
    fn startup_requests_keep_the_order_they_are_written_in() {
        let p = check_src(&format!("{HEADER} invoke /b Ping instance a : A mount /a a invoke /a Ping instance b : B mount /b b invoke /b/x Ping"))
            .unwrap();
        let order: Vec<&str> = p.startup.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(order, ["/b", "/a", "/b/x"]);
    }

    #[test]
    fn requests_resolve_in_the_images_view() {
        let p = check_src(&format!("{HEADER} mount / e  mount /e/out cons  instance e : Echo  invoke /x Ping  write /e/out Line {{ text: \"hi\" }}")).unwrap();
        let view = [MountDecl { at: "/".into(), target: 1, read_only: false }, MountDecl { at: "/e/out".into(), target: 0, read_only: false }];
        assert_eq!(p.view, view);
        let paths: Vec<&str> = p.startup.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["/x", "/e/out"], "kept as written, and resolved again when the image runs");

        let e = err(&format!("{HEADER} mount /e e readonly  instance e : Echo  invoke /e Ping"));
        assert!(e.contains("`/e` is read-only in the image's view, so it can't take `invoke`"), "{e}");
        assert!(check_src(&format!("{HEADER} mount /e e readonly  instance e : Echo  read /e")).is_ok());
        let e = err(&format!("{HEADER} mount /c cons  mount /c/ cons"));
        assert!(e.contains("`/c` is already mounted in the image's view"), "{e}");
        let e = err(&format!("{HEADER} mount /g ghost"));
        assert!(e.contains("unknown instance `ghost`"), "{e}");
    }

    #[test]
    fn bytes_fields_take_string_literals_as_utf8() {
        let p = check_src(&format!("{HEADER} instance e : Echo mount /e e write /e Blob {{ data: \"hé\" }}")).unwrap();
        let body = p.startup[0].body.as_ref().unwrap().message().unwrap();
        let data = body.fields.get_binary_generic("data").unwrap();
        assert_eq!(data, "hé".as_bytes());
    }

    #[test]
    fn environment_objects_take_config_from_the_image() {
        let p = check_src(
            r#"instance net : Environment.Tcp mount /net net control /net Listen { address: "127.0.0.1:0" }
            instance fs : Environment.Files mount /fs fs control /fs FilesRoot { path: "public" }"#,
        )
        .unwrap();
        assert_eq!(p.instances[0].kind, Kind::Tcp);
        let config = p.instances[0].config.as_ref().unwrap().message().unwrap();
        assert_eq!(config.fields.get_str("address").unwrap(), "127.0.0.1:0");
        assert_eq!(p.instances[1].kind, Kind::Files);
        assert!(p.startup.is_empty(), "config isn't a request");

        let e = err("instance net : Environment.Tcp");
        assert!(e.contains("needs exactly one config message: mount it in the image's view and `control` its root with a Listen"), "{e}");
        let e = err(r#"instance net : Environment.Tcp mount /net net control /net Listen { address: "a" } control /net Listen { address: "b" }"#);
        assert!(e.contains("exactly one"), "{e}");
        let e = err(r#"instance c : Environment.Console mount /c c control /c Listen { address: "a" }"#);
        assert!(e.contains("takes no config"), "{e}");
        let e = err(r#"instance net : Environment.Tcp mount /net net control /net/x Listen { address: "a" }"#);
        assert!(e.contains("Environment.Tcp is configured at its root, and `/net/x` is `/x` in it"), "{e}");
        let e = err("instance c : Environment.Stdin");
        assert!(e.contains("unknown built-in type `Environment.Stdin`"), "{e}");
    }

    #[test]
    fn packed_files_take_includes_and_nothing_else() {
        let p = check_src(
            r#"instance files : Packed.Files { include "public/" include "a.txt" at /b//c/ }
            instance site : Site { mount /files files }"#,
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
        let e = err(r#"instance f : Packed.Files mount /f f control /f FilesRoot { path: "." }"#);
        assert!(e.contains("takes no config"), "{e}");
        let e = err(r#"instance a : A instance f : Packed.Files { mount /a a }"#);
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
            instance net : Environment.Tcp mount /net net control /net listen
            instance e : Echo mount /e e write /e/config config"#
        ))
        .unwrap();
        assert_eq!(p.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["listen", "config"]);
        assert_eq!(p.slots[1].defaults, bson::doc! { "name": "x", "count": 3 }, "defaults may be partial");
        assert_eq!(p.instances[1].config, Some(Arg::Slot(0)));
        assert_eq!(p.startup[0].body, Some(Arg::Slot(1)));

        let inst = |decls: &str, request: &str| format!("{HEADER} {decls} instance e : Echo mount /e e {request}");
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
        let e = err(&inst("require g : Greeting", "write /e g { text: \"x\" }"));
        assert!(e.contains("`g` is a slot; its fields come from the host, so use it alone"), "{e}");
        let e = err("require t : Text instance net : Environment.Tcp mount /net net control /net t");
        assert!(e.contains("needs exactly one config message"), "{e}");
    }

    #[test]
    fn runtime_types_can_only_be_redeclared_exactly() {
        let dir = std::env::temp_dir().join(format!("othismo_envshape_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m.iface"), "message Listen { port: i32 }\nmessage Line { text: string }").unwrap();
        let mut none = |_: &str| -> Result<Header> { bail!("no modules") };
        let ok = image::parse(r#"import { Line } from "./m" import { Text } from env"#).unwrap();
        assert!(check(&ok, &dir, &mut none).is_ok());
        let bad = image::parse(r#"import { Listen } from "./m.iface""#).unwrap();
        let e = format!("{:#}", check(&bad, &dir, &mut none).unwrap_err());
        let missing = image::parse(r#"import { Nope } from env"#).unwrap();
        let e2 = format!("{:#}", check(&missing, &dir, &mut none).unwrap_err());
        std::fs::remove_dir_all(&dir).ok();
        assert!(e.contains("`Listen` is a runtime type and must be declared as { address: string }"), "{e}");
        assert!(e2.contains("env has no message type `Nope`"), "{e2}");
    }

    #[test]
    fn types_are_imported_from_headers() {
        let greeter = "message Greeting { prefix: string } object Greeter { serves { /greeting write Greeting } }";
        let p = check_with(
            r#"import { Greeting } from Greeter
            instance g : Greeter mount /g g write /g/greeting Greeting { prefix: "hi" }"#,
            &[("Greeter", greeter)],
        )
        .unwrap();
        assert!(p.messages.contains_key("Greeting"));
        assert_eq!(p.interfaces["Greeter"].serves[0].path.as_str(), "/greeting");
        assert!(p.interfaces.contains_key("Greeter"));

        // A module's types are in the program without being imported, but the image can't write them.
        let p = check_with("instance g : Greeter", &[("Greeter", greeter)]).unwrap();
        assert!(p.messages.contains_key("Greeting"));
        let e = err_with(r#"instance g : Greeter mount /g g write /g/greeting Greeting { prefix: "hi" }"#, &[("Greeter", greeter)]);
        assert!(e.contains("unknown message type `Greeting`; is it imported?"), "{e}");

        let e = err(r#"import { Line } from "othismo/env""#);
        assert!(e.contains("imported `from env`"), "{e}");
        let e = err(r#"import { Ping } from "./messages.ts""#);
        assert!(e.contains("types are imported from headers"), "{e}");
        let e = err("instance g : Ghost");
        assert!(e.contains("instance `g`") && e.contains("no module Ghost"), "{e}");
    }

    #[test]
    fn a_type_is_its_name_and_fields() {
        let a = "message Point { x: i32 } object A {}";
        let b = "message Point { x: f64 } object B {}";
        let e = err_with("instance a : A instance b : B", &[("A", a), ("B", b)]);
        assert!(e.contains("`Point` is declared one way by A and another by B; an image can't use both"), "{e}");
        let same = "message Point { x: i32 } object B {}";
        assert!(check_with("instance a : A instance b : B", &[("A", a), ("B", same)]).is_ok());
    }

    #[test]
    fn startup_requests_must_be_served() {
        let greeter = "import { Text } from env object G { serves { /greeting write Text  /hosts/{name} control Text, control } }";
        let ok = r#"instance g : G mount /g g write /g/greeting Text { text: "hi" } control /g/hosts/* Text { text: "/www" } control /g/hosts/a"#;
        assert!(check_with(ok, &[("G", greeter)]).is_ok());
        let e = err_with("instance g : G mount /g g write /g/nothing", &[("G", greeter)]);
        assert!(e.contains("`g` serves nothing at `/nothing`"), "{e}");
        let e = err_with("instance g : G mount /g g read /g/greeting", &[("G", greeter)]);
        assert!(e.contains("`g` doesn't take `read` at `/greeting`; it serves `/greeting write Text`"), "{e}");
        let e = err_with("instance g : G mount /g g write /g/greeting", &[("G", greeter)]);
        assert!(e.contains("doesn't take `write` at `/greeting`"), "{e}");
        let e = err_with(r#"instance g : G mount /g g write /g/greeting Line { text: "x" }"#, &[("G", greeter)]);
        assert!(e.contains("doesn't take `write Line` at `/greeting`"), "{e}");
        let e = err(r#"instance net : Environment.Tcp mount /net net control /net Listen { address: "a" } read /net/a/b"#);
        assert!(e.contains("`net` serves nothing at `/a/b`"), "built-ins have headers too: {e}");
    }

    #[test]
    fn mounts_must_provide_what_is_needed_there() {
        let echo = "import { Line } from env object Echo { needs /dev/cons { / watch -> Line*, write Line } }";
        let modules = [("Echo", echo)];
        assert!(check_with(&format!("{HEADER} instance e : Echo {{ mount /dev/cons cons }}"), &modules).is_ok());

        let e = err_with(&format!("{HEADER} instance e : Echo"), &modules);
        assert!(e.contains("`e` needs something mounted at `/dev/cons`"), "{e}");
        let e = err_with(&format!("{HEADER} instance e : Echo {{ mount /dev/cons cons readonly }}"), &modules);
        assert!(e.contains("`e` needs `/ write Line` at `/dev/cons`, but `cons` is mounted there read-only"), "{e}");
        let files = r#"instance f : Environment.Files mount /f f control /f FilesRoot { path: "." }"#;
        let e = err_with(&format!("{HEADER} {files} instance e : Echo {{ mount /dev/cons f }}"), &modules);
        assert!(e.contains("`e` needs `/ watch -> Line*` at `/dev/cons`, and `f` serves `/ control FilesRoot, read -> Data, stat -> Info, list -> Entry*`, `/** read -> Data, stat -> Info, list -> Entry*`"), "{e}");
    }

    #[test]
    fn needs_under_a_pattern_cover_every_mount_there() {
        let http = "import { Accepted, Data, Info, Text } from env
            object Http {
              needs /net/tcp {
                /address  read -> Text
                /accept   watch -> Accepted*
                /{conn}   watch -> Data*, write Data, remove
              }
              needs /** { /** read -> Opaque, stat -> Info }
            }";
        let site = "import { Data, Info, Text } from env object Site { serves { /hits read -> Text  /** read -> Data, stat -> Info } }";
        let strict = "import { Data } from env object Strict { needs /** { /** read -> Data } }";
        let modules = [("Http", http), ("Site", site), ("Strict", strict)];
        let net = r#"instance net : Environment.Tcp mount /net net control /net Listen { address: "127.0.0.1:0" }"#;
        let img = format!("{net} instance site : Site instance http : Http {{ mount /net/tcp net  mount /www site  mount /other site }}");
        assert!(check_with(&img, &modules).is_ok(), "{:#}", check_with(&img, &modules).unwrap_err());

        // /hits answers a read with Text, where Strict needs Data everywhere.
        let e = err_with("instance site : Site instance s : Strict { mount /www site }", &modules);
        assert!(e.contains("`s` needs `/** read -> Data` at `/www`, and `site` serves `/hits read -> Text`"), "{e}");
    }

    #[test]
    fn describes_views_and_reach() {
        let p = check_src(
            r#"instance cons : Environment.Console
            instance files : Environment.Files mount /files files control /files FilesRoot { path: "." }
            instance site : Site { mount /files files readonly }
            instance http : Http { mount /www site  mount /dev/cons cons }"#,
        )
        .unwrap();
        assert_eq!(
            describe(&p, &|_| None),
            "the image sees /files → files\n\n\
             cons   sees nothing\n\
             files  sees nothing\n\
             site   sees /files → files ro\n       can reach at most: files ro\n\
             http   sees /www → site, /dev/cons → cons\n       can reach at most: cons rw, files ro (via site), site rw\n"
        );
    }
}
