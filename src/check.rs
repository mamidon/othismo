//! Resolves and validates a parsed image into a [`Program`] the runtime can run.

use crate::iface::{self, Header, Loader, Object, Trait};
use crate::image::{self, Ident, Image, Literal, SlotDecl, Value};
use crate::messages::{FieldType, MessageType};
use crate::ns::{self, Namespace, Status, Verb};
use crate::pattern::Pattern;
use crate::payload::{self, Payload};
use crate::wire::{self, Message};
use anyhow::{Context, Result, anyhow, bail};
use bson::{Bson, Document};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// The traits the runtime implements itself. An image declares an instance
/// of one by the trait, and the runtime picks how to run it.
pub const CONSOLE: &str = "Console";
pub const LISTENER: &str = "Listener";
pub const FILES: &str = "Files";
pub const NATIVE: &[&str] = &[CONSOLE, LISTENER, FILES];
/// The one built-in written by its own name: it touches nothing on the host,
/// so there is nothing to choose.
pub const PACKED: &str = "Packed.Files";

/// What an imported image's instance of one of the runtime's traits is run
/// as: in memory, with a trait of its own that the importing image can use
/// to drive it from the other side.
const MEMORY: &[(&str, Option<&str>)] = &[(CONSOLE, Some("Terminal")), (LISTENER, Some("Dialer")), (FILES, None)];

/// The runtime's objects that are configured, by the image's write of their
/// root, with text saying what each one is.
const CONFIGURED: &[&str] = &[LISTENER, FILES];

/// What the runtime's objects were called when images named them directly.
const RENAMED: &[(&str, &str)] =
    &[("Environment.Console", CONSOLE), ("Environment.Tcp", LISTENER), ("Environment.Files", FILES)];

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
    /// The files the image was checked from: the `.oth` files: the image and the headers it
    /// imports from files.
    /// Not kept in a built image, which has copies under its sources.
    #[serde(skip)]
    pub sources: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    /// A module name, one of the runtime's traits, such as `Listener`, or
    /// `Packed.Files`; or for one of those traits run in memory, such as
    /// `Memory.Listener`.
    pub type_name: String,
    pub kind: Kind,
    /// The directory of the image that declared it, relative to this one's,
    /// which its relative paths (module, includes, files root) start from.
    pub dir: String,
    /// One of the runtime's objects in an imported image, which runs in
    /// memory and touches nothing on the host.
    pub in_memory: bool,
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

/// What the image gives in a startup request: a message, a payload, or a
/// slot standing for either.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Arg {
    Message(Message),
    Payload(Payload),
    Slot(usize),
}

impl Arg {
    /// The message, once any slot has been bound (see [`crate::slots::bind`]).
    pub fn message(&self) -> Result<&Message> {
        match self {
            Arg::Message(m) => Ok(m),
            Arg::Payload(p) => bail!("a {} payload isn't a message", p.ty),
            Arg::Slot(i) => bail!("slot {i} has no value; the image hasn't been bound"),
        }
    }

    /// What it carries, as a payload, once any slot has been bound: a
    /// message is a BSON one.
    pub fn payload(&self) -> Result<Payload> {
        match self {
            Arg::Message(m) => Ok(Payload::new(payload::BSON, m.encode())),
            Arg::Payload(p) => Ok(p.clone()),
            Arg::Slot(i) => bail!("slot {i} has no value; the image hasn't been bound"),
        }
    }

    /// Its text, for a text payload, once any slot has been bound.
    pub fn text(&self) -> Result<String> {
        let p = self.payload()?;
        if !p.is_text() {
            bail!("expected text, and it is {}", p.ty);
        }
        String::from_utf8(p.bytes).context("its text isn't UTF-8")
    }
}

/// `require <name> : <Type> { defaults }` or `require <name> : <content
/// type> = "<default>"`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    pub kind: SlotKind,
    /// Once bound: the value, and where it, or each of its fields, came from.
    #[serde(skip)]
    pub value: Option<Arg>,
    #[serde(skip)]
    pub origins: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SlotKind {
    /// A message, with the image's defaults: some, all or none of the type's fields.
    Message { type_name: String, defaults: Document },
    /// A payload of a content type, given as a string.
    Payload { ty: String, default: Option<String> },
}

impl Slot {
    /// The content type of what it stands for.
    pub fn content_type(&self) -> &str {
        match &self.kind {
            SlotKind::Message { .. } => payload::BSON,
            SlotKind::Payload { ty, .. } => ty,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    /// A module: its object's name, and where the module is, relative to
    /// the root image's directory.
    Wasm { type_name: String, module: String },
    Console,
    /// Listens on the address its config gives.
    Tcp,
    /// Serves files under the directory its config gives. A relative path
    /// is relative to the image.
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

fn err_at(id: &Ident, msg: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{}: {msg}", id.pos)
}

/// One `.oth` file of an image: the root, or one it imports whole.
struct Unit {
    image: Image,
    /// Its directory, relative to the root's: where its relative paths
    /// (includes, a files root) start.
    rel: PathBuf,
    /// Imported whole by another, so the runtime's objects in it run in memory.
    imported: bool,
    /// What its names mean: its declarations and imports, resolved.
    scope: Header,
}

/// What an instance's type turned out to be.
enum TypeRef {
    /// One of the runtime's own, by its name: a trait it implements, such
    /// as `Listener`, or `Packed.Files`.
    Builtin(String),
    /// A module's object: the interface the module itself carries, the
    /// module, relative to the root's directory, and its message types.
    Wasm { object: Object, module: String, messages: BTreeMap<String, MessageType> },
}

/// Reads, parses and checks the image at `path`, with the images it imports
/// whole and everything it imports declarations from, reading each module's
/// header from the module.
pub fn load(path: &Path) -> Result<Program> {
    load_with(path, &mut Loader::default())
}

/// [`load`], reading every file through `loader`.
pub fn load_with(path: &Path, loader: &mut Loader) -> Result<Program> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut units = vec![];
    gather(path, Path::new(""), 0, loader, &mut units)?;
    let mut program = check(&units, dir, loader).with_context(|| format!("checking {}", path.display()))?;
    program.sources = loader.sources.clone();
    Ok(program)
}

/// Adds the image at `path` to `units`, after every image it imports whole,
/// so that their requests are made first. `rel` is `path`'s directory
/// relative to the root's.
fn gather(path: &Path, rel: &Path, depth: usize, loader: &mut Loader, units: &mut Vec<Unit>) -> Result<()> {
    if depth > 16 {
        bail!("images import each other in a loop");
    }
    let image = loader.image(path)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    for imp in &image.images {
        let child = Path::new(&imp.path);
        let child_rel = iface::join(rel, &child.parent().unwrap_or(Path::new("")).to_string_lossy());
        gather(&iface::join(dir, &imp.path), &child_rel, depth + 1, loader, units)
            .with_context(|| format!("{}: importing {}", imp.pos, imp.path))?;
    }
    let scope = loader.resolve(&image.header, path)?;
    units.push(Unit { image, rel: rel.to_path_buf(), imported: depth > 0, scope });
    Ok(())
}

/// What the instance type `ty` names in `unit`: an object it declares or
/// imports, or one of the runtime's traits it imports from env. A module's
/// interface is the header inside it, which must be the one it was built
/// from. `dir` is the root's directory.
fn instance_type(unit: &Unit, ty: &Ident, dir: &Path, loader: &mut Loader) -> Result<TypeRef> {
    if let Some(o) = unit.scope.object_named(&ty.name) {
        let Some(origin) = &o.origin else { return Ok(TypeRef::Builtin(o.name.clone())) };
        let module = if origin.extension().is_some_and(|e| e == "wasm") { origin.clone() } else { origin.with_extension("wasm") };
        if loader.read(&module).is_err() {
            bail!("cannot read {} (did you compile {}.ts?)", module.display(), o.name);
        }
        let header = loader.header(&module)?;
        iface::for_module(&header, &o.name).with_context(|| format!("{} has the wrong header", module.display()))?;
        let object = header.object(&o.name).cloned().expect("for_module found it");
        if !object.same_shape(o) {
            bail!("{} is stale: the header inside it isn't {}'s; rebuild it", module.display(), origin.display());
        }
        let rel = module.strip_prefix(dir).unwrap_or(&module).to_string_lossy().into_owned();
        return Ok(TypeRef::Wasm { object, module: rel, messages: header.messages });
    }
    if let Some(t) = unit.scope.traits.get(&ty.name) {
        if NATIVE.contains(&t.name.as_str()) && iface::env().traits.get(&t.name) == Some(t) {
            return Ok(TypeRef::Builtin(t.name.clone()));
        }
        let runs = NATIVE.join(", ");
        bail!(err_at(ty, format!("`{}` is a shape the runtime has no implementation of; it runs {runs}, and {PACKED}", ty.name)));
    }
    if let Some((_, now)) = RENAMED.iter().find(|(old, _)| *old == ty.name) {
        bail!(err_at(ty, format!("the runtime's objects are declared by their traits: `{now}`, imported from env")));
    }
    if NATIVE.contains(&ty.name.as_str()) || ty.name == PACKED {
        bail!(err_at(ty, format!("`{}` is the runtime's; import it first: `import {{ {} }} from env`", ty.name, ty.name)));
    }
    bail!(err_at(ty, format!("unknown object `{}`; is it imported?", ty.name)))
}

/// Validates the image made of `units`, the root last: its messages against
/// their types, and its wiring against its objects' interfaces. `dir` is the
/// root's directory, and `loader` reads modules' headers.
fn check(units: &[Unit], dir: &Path, loader: &mut Loader) -> Result<Program> {
    // Everything every file declares, with the file it is in.
    fn all<'u, T>(units: &'u [Unit], of: fn(&'u Image) -> &'u [T]) -> Vec<(usize, &'u T)> {
        units.iter().enumerate().flat_map(|(u, unit)| of(&unit.image).iter().map(move |t| (u, t))).collect()
    }
    let decls = all(units, |i| &i.instances);
    let requires = all(units, |i| &i.requires);
    let requests = all(units, |i| &i.requests);
    let top_mounts: Vec<image::Mount> = all(units, |i| &i.mounts).into_iter().map(|(_, m)| m.clone()).collect();

    let mut types: Vec<TypeRef> = vec![];
    let mut modules: BTreeMap<String, String> = BTreeMap::new();
    let mut sets: Vec<(String, BTreeMap<String, MessageType>)> = vec![("the runtime".into(), iface::env().messages.clone())];
    for &(u, inst) in &decls {
        let ty = &inst.type_name;
        let t = instance_type(&units[u], ty, dir, loader).with_context(|| format!("{}: instance `{}`", ty.pos, inst.name.name))?;
        if let TypeRef::Wasm { object, module, messages } = &t {
            if let Some(prev) = modules.insert(object.name.clone(), module.clone())
                && prev != *module
            {
                bail!("{}: `{}` would come from both {prev} and {module}; an image can't use two modules of one name", ty.pos, object.name);
            }
            sets.push((object.name.clone(), messages.clone()));
        }
        types.push(t);
    }
    sets.extend(units.iter().map(|u| ("the image's imports".to_string(), u.scope.messages.clone())));
    let messages = all_messages(&sets)?;

    // What each instance runs as: a module's object, or one of the
    // runtime's, which in an imported image runs in memory.
    let mut interfaces: BTreeMap<String, Object> = BTreeMap::new();
    let mut type_names = vec![];
    for (&(u, _), t) in decls.iter().zip(&types) {
        let o = match t {
            TypeRef::Builtin(name) => builtin(name, units[u].imported).expect("only the runtime's are Builtin"),
            TypeRef::Wasm { object, .. } => object.clone(),
        };
        type_names.push(o.name.clone());
        interfaces.insert(o.name.clone(), o);
    }

    let mut slots: Vec<Slot> = vec![];
    for &(u, req) in &requires {
        let scope = &units[u].scope.messages;
        let name = &req.name.name;
        if scope.contains_key(name) {
            bail!(err_at(&req.name, format!("`{name}` is a message type; a slot needs a name of its own")));
        }
        if slots.iter().any(|s| s.name == *name) {
            bail!(err_at(&req.name, format!("slot `{name}` is declared twice")));
        }
        let kind = match &req.slot {
            SlotDecl::Message(body) => {
                let (type_name, defaults) = body_fields(scope, body, false)?;
                SlotKind::Message { type_name, defaults }
            }
            SlotDecl::Payload { ty, default } => SlotKind::Payload { ty: content_type(ty)?, default: default.clone() },
        };
        slots.push(Slot { name: name.clone(), kind, value: None, origins: vec![] });
    }

    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, (_, inst)) in decls.iter().enumerate() {
        if index.insert(&inst.name.name, i).is_some() {
            bail!(err_at(&inst.name, format!("instance `{}` is declared twice", inst.name.name)));
        }
    }

    // The image's requests resolve in its own view, as an instance's do in
    // its. Nothing has exposed anything yet, so this is where they land.
    let view = mount_decls(&top_mounts, &index, "the image's view")?;
    let image_view = decls.len();
    let mut image_ns = Namespace::new(image_view + 1);
    for m in &view {
        image_ns.mount(image_view, &m.at, m.target, m.read_only);
    }
    // (the instance it lands in, the path there, the path in the image's view)
    let mut addressed: Vec<(usize, String, String)> = vec![];
    for (_, r) in &requests {
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
    // A configurable built-in object's config is the image's write of its
    // root, taken before the object is made rather than sent as a request.
    let is_config = |to: usize, verb: Verb, path: &str| {
        verb == Verb::Write && path == "/" && matches!(&types[to], TypeRef::Builtin(n) if CONFIGURED.contains(&n.as_str()))
    };

    let mut instances = vec![];
    for (i, &(u, inst)) in decls.iter().enumerate() {
        let unit = &units[u];
        let made_of: Vec<(&image::Request, &str, usize)> = requests
            .iter()
            .zip(&addressed)
            .filter(|(_, (to, ..))| *to == i)
            .map(|(&(ru, r), (_, p, _))| (r, p.as_str(), ru))
            .collect();
        let mut config = None;
        let type_name = type_names[i].clone();
        let obj = interfaces.get(&type_name);
        if !matches!(&types[i], TypeRef::Builtin(n) if n == PACKED)
            && let Some(inc) = inst.includes.first()
        {
            bail!("{}: `include` packs files into a {PACKED} instance, and `{}` is a {}", inc.pos, inst.name.name, inst.type_name.name);
        }
        let configs: Vec<&(&image::Request, &str, usize)> = made_of.iter().filter(|(r, path, _)| is_config(i, r.verb, path)).collect();
        let kind = match &types[i] {
            TypeRef::Builtin(builtin) => {
                let name = type_name.as_str();
                if let Some(m) = inst.mounts.first() {
                    bail!(err_at(&m.target, format!("{name} has no view to mount into; it only serves")));
                }
                if let Some(what) = config_what(builtin) {
                    let needs = || {
                        err_at(
                            &inst.name,
                            format!("{name} needs exactly one config: mount it in the image's view and `write` its root with {what}"),
                        )
                    };
                    let [(c, _, cu)] = configs.as_slice() else { bail!(needs()) };
                    let arg = body_arg(&units[*cu].scope.messages, &slots, c.body.as_ref().ok_or_else(needs)?)?;
                    if !payload::fits("text/*", arg_type(&slots, &arg)) {
                        bail!(needs());
                    }
                    config = Some(arg);
                }
                builtin_kind(builtin, &inst.includes)?
            }
            TypeRef::Wasm { object, module, .. } => Kind::Wasm { type_name: object.name.clone(), module: module.clone() },
        };

        let mounts = mount_decls(&inst.mounts, &index, &format!("`{}`'s view", inst.name.name))?;

        if let Some(obj) = obj {
            let made_of: Vec<(&image::Request, &str, usize)> =
                made_of.iter().filter(|(r, path, _)| !is_config(i, r.verb, path)).copied().collect();
            check_requests(inst, obj, &made_of, units, &slots)?;
            check_mounts(&type_names, inst, obj, &index, &interfaces)?;
        }
        let in_memory = unit.imported && matches!(&types[i], TypeRef::Builtin(n) if NATIVE.contains(&n.as_str()));
        let dir = unit.rel.to_string_lossy().into_owned();
        instances.push(Instance { name: inst.name.name.clone(), type_name, kind, dir, in_memory, config, mounts });
    }

    let mut startup = vec![];
    for (&(u, r), (to, path, full)) in requests.iter().zip(&addressed) {
        if is_config(*to, r.verb, path) {
            continue;
        }
        let body = r.body.as_ref().map(|b| body_arg(&units[u].scope.messages, &slots, b)).transpose()?;
        startup.push(Startup { verb: r.verb, path: full.clone(), body });
    }

    Ok(Program { messages, interfaces, slots, instances, view, startup, sources: vec![] })
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
    requests: &[(&image::Request, &str, usize)],
    units: &[Unit],
    slots: &[Slot],
) -> Result<()> {
    let name = &inst.name.name;
    for &(r, path, u) in requests {
        let body = r.body.as_ref().map(|b| body_arg(&units[u].scope.messages, slots, b)).transpose()?;
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
    type_names: &[String],
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
        let target = index[m.target.name.as_str()];
        let Some(served) = interfaces.get(&type_names[target]) else { continue };
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

/// The interface of the built-in type `name`, if it is one: one of the traits
/// the runtime implements, as an object serving the trait's tree, or
/// Packed.Files. One of the traits `imported` is run in memory, and
/// serves its other side too.
fn builtin(name: &str, imported: bool) -> Option<Object> {
    let env = iface::env();
    if name == PACKED {
        return env.object(PACKED).cloned();
    }
    let mut o = env.traits.get(name).filter(|_| NATIVE.contains(&name)).map(Trait::as_object)?;
    if imported {
        o.name = format!("Memory.{name}");
        if let Some((_, Some(other))) = MEMORY.iter().find(|(t, _)| *t == name) {
            o.serves.extend(env.traits[*other].entries.iter().cloned());
            o.traits.push(other.to_string());
        }
    }
    Some(o)
}

/// What the runtime's object `name` is configured with, whether it runs on
/// the host or in memory, for errors; `None` if it takes no config.
fn config_what(name: &str) -> Option<&'static str> {
    match name {
        LISTENER => Some("the address to listen on, e.g. `\"127.0.0.1:8080\"`"),
        FILES => Some("the directory to serve, e.g. `\"public\"`"),
        _ => None,
    }
}


/// How the runtime implements the built-in type `name`. A `Packed.Files`
/// serves its `includes`.
fn builtin_kind(name: &str, includes: &[image::Include]) -> Result<Kind> {
    Ok(match name {
        CONSOLE => Kind::Console,
        LISTENER => Kind::Tcp,
        FILES => Kind::Files,
        PACKED => {
            let mut decls = vec![];
            for inc in includes {
                let at = match &inc.at {
                    Some(p) => Some(
                        ns::mount_point(&p.path).map_err(|_| anyhow!("{}: `{}` is not a clean absolute path", p.pos, p.path))?,
                    ),
                    None => None,
                };
                decls.push(IncludeDecl { source: inc.source.clone(), at });
            }
            Kind::Packed { includes: decls }
        }
        other => bail!("the runtime doesn't implement {other}"),
    })
}

/// Every message type in the image: the runtime's, every module's and the
/// ones its files can name. A type is its name and fields, so two
/// declarations of one name must agree. `sets` names where each set is from.
fn all_messages(sets: &[(String, BTreeMap<String, MessageType>)]) -> Result<BTreeMap<String, MessageType>> {
    let mut out: BTreeMap<String, MessageType> = BTreeMap::new();
    let mut from: BTreeMap<String, String> = BTreeMap::new();
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
                    from.insert(name.clone(), origin.clone());
                }
            }
        }
    }
    Ok(out)
}

/// A startup request's literal: a message, a slot's name standing alone, or
/// a payload.
fn body_arg(messages: &BTreeMap<String, MessageType>, slots: &[Slot], literal: &Literal) -> Result<Arg> {
    let body = match literal {
        Literal::Message(body) => body,
        Literal::Text { ty, text, .. } => {
            let ty = ty.as_ref().map(content_type).transpose()?.unwrap_or_else(|| payload::TEXT.to_string());
            return Ok(Arg::Payload(Payload::new(ty, text.as_bytes())));
        }
    };
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

/// The content type of what `arg` carries.
fn arg_type<'a>(slots: &'a [Slot], arg: &'a Arg) -> &'a str {
    match arg {
        Arg::Message(_) => payload::BSON,
        Arg::Payload(p) => &p.ty,
        Arg::Slot(i) => slots[*i].content_type(),
    }
}

/// The content type an image writes, which must be well-formed.
fn content_type(id: &Ident) -> Result<String> {
    if !payload::valid(&id.name) {
        bail!(err_at(id, format!("`{}` isn't a well-formed content type", id.name)));
    }
    Ok(id.name.clone())
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
        export message Greeting { text: string }
        export message Config { name: string, count: u8, ratio: f64, on: bool }
        export message Ping {}
        export message Blob { data: bytes }
    "#;

    /// `src` with every declaration exported.
    fn exported(src: &str) -> String {
        let mut out = src.to_string();
        for kw in ["message ", "protocol ", "trait ", "object "] {
            out = out.replace(&format!("export {kw}"), kw).replace(kw, &format!("export {kw}"));
        }
        out
    }

    /// A module whose header is `src`, which can only import from env.
    fn module(src: &str) -> Vec<u8> {
        let file = iface::parse(src).unwrap();
        let header = iface::resolve(&file, &mut |_| Ok(iface::env().clone()), &|_| unreachable!()).unwrap();
        iface::embed(&wat::parse_str("(module)").unwrap(), &header).unwrap()
    }

    /// Every request there is, of every verb, with a payload of any type or
    /// none, answered with anything: what a permissive module serves.
    pub(crate) fn anything() -> String {
        let reqs: Vec<String> = ns::VERBS
            .iter()
            .flat_map(|v| match v {
                Verb::Stat | Verb::List => vec![v.to_string()],
                _ => vec![format!("{v} -> */**"), format!("{v} */* -> */**")],
            })
            .collect();
        reqs.join(", ")
    }

    /// The header of a module that needs nothing and serves anything, anywhere.
    fn permissive(name: &str) -> String {
        format!("object {name} {{ serves {{ /** {} }} }}", anything())
    }

    /// Checks `img` as /check/test.oth, beside MESSAGES in messages.oth. Any
    /// instance type it doesn't import is imported for it at the top: from
    /// env if it is the runtime's, and otherwise from ./<Type>.oth, which
    /// has its module beside it, with the header in `modules` or else a
    /// permissive one. A `Ghost` has no module.
    fn check_with(img: &str, modules: &[(&str, &str)]) -> Result<Program> {
        let dir = PathBuf::from("/check");
        let mut loader = Loader::default();
        loader.files.insert(dir.join("messages.oth"), MESSAGES.into());
        let parsed = image::parse(img)?;
        let mut imported: Vec<String> =
            parsed.header.imports.iter().flat_map(|i| i.names.iter().map(|n| n.local.name.clone())).collect();
        let (mut prelude, mut from_env) = (String::new(), vec![]);
        for inst in &parsed.instances {
            let ty = inst.type_name.name.clone();
            if imported.contains(&ty) {
                continue;
            }
            imported.push(ty.clone());
            if iface::env().exports.contains(&ty) {
                from_env.push(ty);
                continue;
            }
            if ty.contains('.') {
                continue;
            }
            let src = exported(&modules.iter().find(|(n, _)| *n == ty).map(|(_, s)| s.to_string()).unwrap_or_else(|| permissive(&ty)));
            if ty != "Ghost" {
                loader.files.insert(dir.join(format!("{ty}.wasm")), module(&src));
            }
            loader.files.insert(dir.join(format!("{ty}.oth")), src.into_bytes());
            prelude.push_str(&format!("import {{ {ty} }} from \"./{ty}.oth\" "));
        }
        if !from_env.is_empty() {
            prelude.push_str(&format!("import {{ {} }} from env ", from_env.join(", ")));
        }
        for (ty, src) in modules {
            loader.files.entry(dir.join(format!("{ty}.oth"))).or_insert_with(|| exported(src).into_bytes());
            loader.files.entry(dir.join(format!("{ty}.wasm"))).or_insert_with(|| module(&exported(src)));
        }
        loader.files.insert(dir.join("test.oth"), format!("{prelude}{img}").into_bytes());
        load_with(&dir.join("test.oth"), &mut loader)
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
        import { Greeting, Config, Ping, Blob } from "./messages.oth"
        instance cons : Console
    "#;

    #[test]
    fn accepts_hello_world() {
        let p = check_src(&format!("{HEADER} instance echo : Echo {{ mount /dev/cons cons }}")).unwrap();
        assert_eq!(p.instances.len(), 2);
        assert_eq!(p.instances[0].kind, Kind::Console);
        assert_eq!(p.instances[1].kind, Kind::Wasm { type_name: "Echo".into(), module: "Echo.wasm".into() });
        assert_eq!(p.instances[1].mounts, [MountDecl { at: "/dev/cons".into(), target: 0, read_only: false }]);
        assert!(p.messages.contains_key("Greeting"));
    }

    #[test]
    fn mounts_are_checked() {
        assert!(err(&format!("{HEADER} instance a : A {{ mount /g ghost }}")).contains("unknown instance `ghost`"));
        let e = err(&format!("{HEADER} instance a : A {{ mount /x/../y cons }}"));
        assert!(e.contains("`/x/../y` is not a clean absolute path"), "{e}");
        let e = err(&format!("{HEADER} instance a : A {{ mount /c cons mount /c/ cons }}"));
        assert!(e.contains("`/c` is already mounted"), "{e}");
        let e = err(&format!("{HEADER} instance net : Console {{ mount /c cons }}"));
        assert!(e.contains("only serves"), "{e}");
        // Forward references are fine: views are built once every instance exists.
        let p = check_src(&format!("{HEADER} instance a : A {{ mount /b b readonly }} instance b : B")).unwrap();
        assert_eq!(p.instances[1].mounts[0], MountDecl { at: "/b".into(), target: 2, read_only: true });
    }

    #[test]
    fn startup_requests_are_typed() {
        let inst = |req: &str| format!("{HEADER} instance e : Echo mount /e e {req}");
        let p = check_src(&inst(
            "write /e/config Config { name: \"x\", count: 3, ratio: 2, on: true } create /e Ping remove /e/a//b/",
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
        assert!(err(&inst("create /e Ping { x: 1 }")).contains("no field `x`"));
        assert!(err(&inst("create /e Missing")).contains("unknown message type"));
        assert!(err(&inst("write /e Blob { data: 1 }")).contains("is bytes"));
        assert!(err(&inst("read /e/../x")).contains("not a clean absolute path"));
        assert!(err(&inst("read /x")).contains("nothing is mounted at `/x` in the image's view"));
        assert!(err(&inst("read /")).contains("nothing is mounted at `/` in the image's view"));
        assert!(err(&format!("{HEADER} write /e Ping instance e : Echo")).contains("nothing is mounted at `/e`"));
    }

    #[test]
    fn a_name_is_imported_once_in_a_file() {
        let twice = r#"import { Greeting } from "./messages.oth" import { Greeting } from "./messages.oth""#;
        assert!(format!("{:#}", check_src(twice).unwrap_err()).contains("`Greeting` is already imported"));
        let renamed = r#"import { Greeting, Ping as Pong } from "./messages.oth""#;
        assert!(format!("{:#}", check_src(renamed).unwrap_err()).contains("`Ping` can't be renamed"));
    }

    #[test]
    fn startup_requests_keep_the_order_they_are_written_in() {
        let p = check_src(&format!("{HEADER} write /b Ping instance a : A mount /a a write /a Ping instance b : B mount /b b write /b/x Ping"))
            .unwrap();
        let order: Vec<&str> = p.startup.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(order, ["/b", "/a", "/b/x"]);
    }

    #[test]
    fn requests_resolve_in_the_images_view() {
        let p = check_src(&format!("{HEADER} mount / e  mount /e/out cons  instance e : Echo  write /x Ping  write /e/out \"hi\"")).unwrap();
        let view = [MountDecl { at: "/".into(), target: 1, read_only: false }, MountDecl { at: "/e/out".into(), target: 0, read_only: false }];
        assert_eq!(p.view, view);
        let paths: Vec<&str> = p.startup.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["/x", "/e/out"], "kept as written, and resolved again when the image runs");

        let e = err(&format!("{HEADER} mount /e e readonly  instance e : Echo  write /e Ping"));
        assert!(e.contains("`/e` is read-only in the image's view, so it can't take `write`"), "{e}");
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
            r#"instance net : Listener mount /net net write /net "127.0.0.1:0"
            instance fs : Files mount /fs fs write /fs "public""#,
        )
        .unwrap();
        assert_eq!(p.instances[0].kind, Kind::Tcp);
        assert_eq!(p.instances[0].config.as_ref().unwrap().text().unwrap(), "127.0.0.1:0");
        assert_eq!(p.instances[1].kind, Kind::Files);
        assert!(p.startup.is_empty(), "config isn't a request");

        let e = err("instance net : Listener");
        assert!(e.contains("Listener needs exactly one config: mount it in the image's view and `write` its root with the address to listen on, e.g. `\"127.0.0.1:8080\"`"), "{e}");
        let e = err(r#"instance net : Listener mount /net net write /net "a" write /net "b""#);
        assert!(e.contains("exactly one"), "{e}");
        let e = err(&format!(r#"{HEADER} instance net : Listener mount /net net write /net Ping"#));
        assert!(e.contains("exactly one config"), "it takes text: {e}");
        let p = check_src(r#"instance c : Console mount /c c write /c "printed""#).unwrap();
        assert_eq!(p.startup.len(), 1, "a Console takes no config, so writing it prints");
        let e = err(r#"instance net : Listener mount /net net write /net "a" write /net/x "b""#);
        assert!(e.contains("`net` doesn't take `write` at `/x`"), "{e}");
        let e = err("instance c : Environment.Stdin");
        assert!(e.contains("unknown object `Environment.Stdin`"), "{e}");
        let e = err("instance net : Environment.Tcp");
        assert!(e.contains("the runtime's objects are declared by their traits: `Listener`, imported from env"), "{e}");
        let e = format!("{:#}", load_with(Path::new("/l/test.oth"), &mut {
            let mut l = Loader::default();
            l.files.insert("/l/test.oth".into(), b"instance net : Listener".to_vec());
            l
        }).unwrap_err());
        assert!(e.contains("`Listener` is the runtime's; import it first: `import { Listener } from env`"), "{e}");
        let e = err("instance t : FileTree");
        assert!(e.contains("`FileTree` is a shape the runtime has no implementation of; it runs Console, Listener, Files, and Packed.Files"), "{e}");
    }

    #[test]
    fn the_configured_objects_take_text_at_their_root() {
        for name in CONFIGURED {
            assert!(config_what(name).is_some(), "{name}");
            let root = builtin(name, false).unwrap().serves.into_iter().find(|e| e.path.as_str() == "/").unwrap();
            assert!(root.takes(Verb::Write, Some("text/plain")).is_ok_and(|r| r.operator), "{name}: {root}");
        }
        assert_eq!(config_what(CONSOLE), None);
        assert_eq!(config_what(PACKED), None);
        for name in NATIVE.iter().chain([&PACKED]) {
            assert!(iface::env().traits.contains_key(*name) || iface::env().object(name).is_some(), "env declares {name}");
            assert!(builtin_kind(name, &[]).is_ok(), "the runtime implements {name}");
        }
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
        let e = err(r#"instance f : Packed.Files mount /f f write /f ".""#);
        assert!(e.contains("`f` doesn't take `write` at `/`"), "it takes no config: {e}");
        let e = err(r#"instance a : A instance f : Packed.Files { mount /a a }"#);
        assert!(e.contains("only serves"), "{e}");
        let e = err(r#"instance f : Packed.Dirs"#);
        assert!(e.contains("unknown object `Packed.Dirs`"), "{e}");
    }

    #[test]
    fn slots_stand_in_for_messages() {
        let p = check_src(&format!(
            r#"{HEADER}
            require listen : text/plain
            require config : Config {{ name: "x", count: 3 }}
            instance net : Listener mount /net net write /net listen
            instance e : Echo mount /e e write /e/config config"#
        ))
        .unwrap();
        assert_eq!(p.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["listen", "config"]);
        let SlotKind::Message { defaults, .. } = &p.slots[1].kind else { panic!() };
        assert_eq!(*defaults, bson::doc! { "name": "x", "count": 3 }, "defaults may be partial");
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
        let e = err(&format!("{HEADER} require t : Ping instance net : Listener mount /net net write /net t"));
        assert!(e.contains("needs exactly one config"), "{e}");
    }

    #[test]
    fn types_are_imported_from_headers() {
        let greeter = "message Greeting { prefix: string } object Greeter { serves { /greeting write application/bson } }";
        let p = check_with(
            r#"import { Greeting } from "./Greeter.oth"
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

        let e = err(r#"import { Ping } from "./messages.ts""#);
        assert!(e.contains("/check/messages.ts is neither a `.oth` file nor a module (`.wasm`)"), "{e}");
        let e = err("instance g : Ghost");
        assert!(e.contains("instance `g`") && e.contains("cannot read /check/Ghost.wasm (did you compile Ghost.ts?)"), "{e}");
        let e = err("instance g : Nobody.Here");
        assert!(e.contains("unknown object `Nobody.Here`; is it imported?"), "{e}");

        // Renamed, the object is still itself.
        let p = check_with(r#"import { Greeter as G } from "./Greeter.oth" instance g : G"#, &[("Greeter", greeter)]).unwrap();
        assert_eq!(p.instances[0].type_name, "Greeter");

        // A module must be the one its header was built into.
        let stale = "message Greeting { prefix: string } object Greeter { serves { /greeting write text/plain } }";
        let mut loader = Loader::default();
        loader.files.insert("/s/Greeter.oth".into(), exported(stale).into_bytes());
        loader.files.insert("/s/Greeter.wasm".into(), module(&exported(greeter)));
        loader.files.insert("/s/test.oth".into(), br#"import { Greeter } from "./Greeter.oth" instance g : Greeter"#.to_vec());
        let e = format!("{:#}", load_with(Path::new("/s/test.oth"), &mut loader).unwrap_err());
        assert!(e.contains("/s/Greeter.wasm is stale: the header inside it isn't /s/Greeter.oth's; rebuild it"), "{e}");

        // Or it is imported from the module itself.
        let mut loader = Loader::default();
        loader.files.insert("/m/Greeter.wasm".into(), module(&exported(greeter)));
        loader.files.insert("/m/test.oth".into(), br#"import { Greeter } from "./Greeter.wasm" instance g : Greeter"#.to_vec());
        let p = load_with(Path::new("/m/test.oth"), &mut loader).unwrap();
        assert_eq!(p.instances[0].kind, Kind::Wasm { type_name: "Greeter".into(), module: "Greeter.wasm".into() });
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
        let greeter = "object G { serves { /greeting write application/bson  /hosts/{name} operator write text/plain, operator remove } }";
        let ok = r#"import { Greeting } from "./messages.oth" instance g : G mount /g g write /g/greeting Greeting { text: "hi" } write /g/hosts/* "/www" remove /g/hosts/a"#;
        assert!(check_with(ok, &[("G", greeter)]).is_ok());
        let e = err_with("instance g : G mount /g g write /g/nothing", &[("G", greeter)]);
        assert!(e.contains("`g` serves nothing at `/nothing`"), "{e}");
        let e = err_with("instance g : G mount /g g read /g/greeting", &[("G", greeter)]);
        assert!(e.contains("`g` doesn't take `read` at `/greeting`; it serves `/greeting write application/bson`"), "{e}");
        let e = err_with("instance g : G mount /g g write /g/greeting", &[("G", greeter)]);
        assert!(e.contains("doesn't take `write` at `/greeting`"), "{e}");
        let only_text = "object G { serves { /greeting write text/plain } }";
        let e = err_with(r#"import { Greeting } from "./messages.oth" instance g : G mount /g g write /g/greeting Greeting { text: "x" }"#, &[("G", only_text)]);
        assert!(e.contains("doesn't take `write application/bson` at `/greeting`"), "{e}");
        let texts = "object T { serves { /plain write text/plain  /any write text/* } }";
        let ok = r#"require t : text/plain = "x" instance t : T mount /t t write /t/plain "hi" write /t/any text/html "<p>" write /t/plain t"#;
        assert!(check_with(ok, &[("T", texts)]).is_ok(), "{:#}", check_with(ok, &[("T", texts)]).unwrap_err());
        let e = err_with(r#"instance t : T mount /t t write /t/plain text/html "<p>""#, &[("T", texts)]);
        assert!(e.contains("doesn't take `write text/html` at `/plain`"), "{e}");
        let e = err_with(r#"instance t : T mount /t t write /t/plain text/ "x""#, &[("T", texts)]);
        assert!(e.contains("`text/` isn't a well-formed content type"), "{e}");
        let e = err(r#"instance net : Listener mount /net net write /net "a" read /net/a/b"#);
        assert!(e.contains("`net` serves nothing at `/a/b`"), "built-ins have headers too: {e}");
    }

    #[test]
    fn mounts_must_provide_what_is_needed_there() {
        let echo = "object Echo { needs /dev/cons { / watch -> text/plain*, write text/plain } }";
        let modules = [("Echo", echo)];
        assert!(check_with(&format!("{HEADER} instance e : Echo {{ mount /dev/cons cons }}"), &modules).is_ok());

        let e = err_with(&format!("{HEADER} instance e : Echo"), &modules);
        assert!(e.contains("`e` needs something mounted at `/dev/cons`"), "{e}");
        let e = err_with(&format!("{HEADER} instance e : Echo {{ mount /dev/cons cons readonly }}"), &modules);
        assert!(e.contains("`e` needs `/ write text/plain` at `/dev/cons`, but `cons` is mounted there read-only"), "{e}");
        let files = r#"instance f : Files mount /f f write /f ".""#;
        let e = err_with(&format!("{HEADER} {files} instance e : Echo {{ mount /dev/cons f }}"), &modules);
        assert!(e.contains("`e` needs `/ watch -> text/plain*` at `/dev/cons`, and `f` serves `/ operator write text/plain, read -> */*, stat, list`, `/** read -> */*, stat, list`"), "{e}");
    }

    #[test]
    fn needs_under_a_pattern_cover_every_mount_there() {
        let http = "object Http {
              needs /net/tcp {
                /address        read -> text/plain
                /accept         watch -> text/plain*
                /{conn}/data    watch -> application/octet-stream*, write application/octet-stream
                /{conn}         remove
              }
              needs /** { /** read -> */*, stat }
            }";
        let site = "object Site { serves { /hits read -> text/plain  /** read -> text/html, stat } }";
        let strict = "object Strict { needs /** { /** read -> text/html } }";
        let modules = [("Http", http), ("Site", site), ("Strict", strict)];
        let net = r#"instance net : Listener mount /net net write /net "127.0.0.1:0""#;
        let img = format!("{net} instance site : Site instance http : Http {{ mount /net/tcp net  mount /www site  mount /other site }}");
        assert!(check_with(&img, &modules).is_ok(), "{:#}", check_with(&img, &modules).unwrap_err());

        // /hits answers a read with text/plain, where Strict needs text/html everywhere.
        let e = err_with("instance site : Site instance s : Strict { mount /www site }", &modules);
        assert!(e.contains("`s` needs `/** read -> text/html` at `/www`, and `site` serves `/hits read -> text/plain`"), "{e}");
    }

    #[test]
    fn describes_views_and_reach() {
        let p = check_src(
            r#"instance cons : Console
            instance files : Files mount /files files write /files "."
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
