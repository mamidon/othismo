//! Resolves and validates a parsed image into a [`Program`] the runtime can
//! run. The image is instantiated whole: every image it instantiates is
//! made part of it, with its instances named under the instance's name, and
//! every view, bind and export is worked out before anything runs.

use crate::iface::{self, Header, Loader, Object, Trait};
use syntax::image_definition::{self, Ident, ImageDefinition, Literal, Pos};
use protocol::ns::{self, Namespace, Status, Verb};
use protocol::pattern::Pattern;
use protocol::payload::{self, Payload};
pub use image::program::*;
use anyhow::{Context, Result, anyhow, bail};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use format::{AllSchemas, Formats};
use std::path::{Path, PathBuf};

/// The traits the runtime implements on the host. The image being run gets
/// one by needing it, and the runtime picks how to run it.
pub const CONSOLE: &str = "Console";
pub const LISTENER: &str = "Listener";
pub const FILES: &str = "Files";
pub const NATIVE: &[&str] = &[CONSOLE, LISTENER, FILES];
/// The runtime's objects that touch nothing outside the image, which any
/// image can declare by name.
pub const PACKED: &str = "PackedFiles";
pub const MEMORY_CONSOLE: &str = "MemoryConsole";
pub const MEMORY_LISTENER: &str = "MemoryListener";
pub const IN_MEMORY: &[&str] = &[PACKED, MEMORY_CONSOLE, MEMORY_LISTENER];

/// The runtime's objects that are configured, by the image's write of their
/// root, with text saying what each one is.
const CONFIGURED: &[&str] = &[LISTENER, FILES];

/// What the runtime's objects were called when images named them directly.
const RENAMED: &[(&str, &str)] = &[
    ("Environment.Console", CONSOLE),
    ("Environment.Tcp", LISTENER),
    ("Environment.Files", FILES),
    ("Packed.Files", PACKED),
];

fn err_at(id: &Ident, msg: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{}: {msg}", id.pos)
}

/// A `.oth` file the image uses as an image: the root, or one it instantiates.
struct Unit {
    image: ImageDefinition,
    /// Its directory, relative to the root's: where its relative paths
    /// (includes, a files root) start.
    rel: PathBuf,
    /// Its directory as it is read.
    dir: PathBuf,
    /// What its names mean: its declarations and imports, resolved.
    scope: Header,
}

/// What an instance's type turned out to be.
enum TypeRef {
    /// One of the runtime's own, by its name: a trait it implements, such
    /// as `Listener`, for one of the root's needs, or one of the objects
    /// in memory, such as `PackedFiles`.
    Builtin(String),
    /// A module's object: the interface the module itself carries, and the
    /// module, relative to the root's directory.
    Wasm { object: Object, module: String },
    /// An image, by the unit it is.
    Image(usize),
}

/// Something in an image that is resolved once every view is built: a
/// bind, into an instance's view, or an export, into an image's served tree.
struct Pending {
    /// Whose view the bind is into, or the instance of an image the export
    /// is from.
    node: usize,
    export: bool,
    at: String,
    /// A path in `from`'s view: the instance of an image, or `None` for the root.
    from: Option<usize>,
    path: String,
    read_only: bool,
    pos: Pos,
}

/// An instance, while the image is being instantiated.
struct Node {
    name: String,
    unit: usize,
    ty: TypeRef,
    /// Where it was declared, for errors; `None` for one of the root's needs.
    decl: Option<image_definition::Instance>,
    mounts: Vec<MountDecl>,
    /// Where each of `mounts` is written, to keep them in that order.
    written: Vec<Pos>,
    exports: Vec<MountDecl>,
    config: Option<Arg>,
}

/// What a slot of an image is given: one of the root's, or a literal,
/// encoded once every module's schemas are known.
#[derive(Clone)]
enum Given {
    Arg(Arg),
    Literal { literal: Literal, unit: usize, ty: String, what: String },
}

/// A startup request, with the slots its image was given.
struct Request {
    image: Option<usize>,
    unit: usize,
    request: image_definition::Request,
    slots: BTreeMap<String, Given>,
}

struct Builder<'l> {
    loader: &'l mut Loader,
    /// The root image's directory.
    dir: PathBuf,
    units: Vec<Unit>,
    unit_paths: BTreeMap<PathBuf, usize>,
    nodes: Vec<Node>,
    view: Vec<MountDecl>,
    pending: Vec<Pending>,
    requests: Vec<Request>,
    /// The root's slots.
    slots: Vec<Slot>,
    /// Every module's schemas, and which module each type came from, by
    /// content type.
    messages: AllSchemas,
    message_from: BTreeMap<String, BTreeMap<String, String>>,
    modules: BTreeMap<String, String>,
}

/// Reads, parses and checks the image at `path`, with the images it
/// instantiates and everything its files import, reading each module's
/// header from the module. Its payload literals and slots may be in any of
/// `formats`.
pub fn load(path: &Path, formats: Formats) -> Result<Program> {
    load_with(path, &mut Loader::new(formats))
}

/// [`load`], reading every file through `loader`.
pub fn load_with(path: &Path, loader: &mut Loader) -> Result<Program> {
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut b = Builder {
        loader,
        dir,
        units: vec![],
        unit_paths: BTreeMap::new(),
        nodes: vec![],
        view: vec![],
        pending: vec![],
        requests: vec![],
        slots: vec![],
        messages: AllSchemas::new(),
        message_from: BTreeMap::new(),
        modules: BTreeMap::new(),
    };
    let root = b.unit(path, Path::new(""))?;
    let mut program = b.build(root).with_context(|| format!("checking {}", path.display()))?;
    program.sources = loader.sources.clone();
    Ok(program)
}

impl Builder<'_> {
    /// The unit for the `.oth` file at `path`, read once; `rel` is its
    /// directory relative to the root's.
    fn unit(&mut self, path: &Path, rel: &Path) -> Result<usize> {
        if let Some(&u) = self.unit_paths.get(path) {
            return Ok(u);
        }
        let image = self.loader.image_definition(path)?;
        let scope = self.loader.resolve(&image.header, path)?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        self.units.push(Unit { image, rel: rel.to_path_buf(), dir, scope });
        self.unit_paths.insert(path.to_path_buf(), self.units.len() - 1);
        Ok(self.units.len() - 1)
    }

    fn build(mut self, root: usize) -> Result<Program> {
        // The root's slots are whoever runs it's to give.
        let mut slots = BTreeMap::new();
        for req in self.units[root].image.requires.clone() {
            let name = &req.name.name;
            if self.slots.iter().any(|s| s.name == *name) {
                bail!(err_at(&req.name, format!("slot `{name}` is declared twice")));
            }
            let ty = content_type(&req.ty)?;
            let default = match &req.default {
                None => None,
                Some(Literal::Text { ty: None, text, .. }) => Some(SlotValue::Text(text.clone())),
                Some(Literal::Tree { ty: None, value, .. }) => Some(SlotValue::Tree(value.clone())),
                Some(_) => bail!(err_at(&req.name, format!("slot `{name}`'s default is text or a structured value, of its type"))),
            };
            slots.insert(name.clone(), Given::Arg(Arg::Slot(self.slots.len())));
            self.slots.push(Slot { name: name.clone(), ty, default, value: None, origins: vec![] });
        }
        self.instantiate(root, "", None, &slots, 0)?;
        self.resolve_pending()?;
        self.finish()
    }

    /// Makes `unit` part of the image, with its instances named under
    /// `prefix`, as the instance of an image `image`, or as the root.
    /// `slots` gives each of its slots a value.
    fn instantiate(&mut self, unit: usize, prefix: &str, image: Option<usize>, slots: &BTreeMap<String, Given>, depth: usize) -> Result<()> {
        if depth > 16 {
            bail!("images instantiate each other in a loop");
        }
        let img = self.units[unit].image.clone();
        if image.is_none() {
            self.root_needs(unit)?;
        }
        // Every instance first, so that mounts can name any of them.
        let mut local: HashMap<String, usize> = HashMap::new();
        for inst in &img.instances {
            let ty = self.instance_type(unit, &inst.type_name).with_context(|| format!("{}: instance `{}`", inst.type_name.pos, inst.name.name))?;
            if local.insert(inst.name.name.clone(), self.nodes.len()).is_some() {
                bail!(err_at(&inst.name, format!("instance `{}` is declared twice", inst.name.name)));
            }
            let name = format!("{prefix}{}", inst.name.name);
            self.nodes.push(Node { name, unit, ty, decl: Some(inst.clone()), mounts: vec![], written: vec![], exports: vec![], config: None });
        }
        let target = |m: &image_definition::Mount| -> Result<usize> {
            local.get(&m.target.name).copied().ok_or_else(|| err_at(&m.target, format!("unknown instance `{}`", m.target.name)))
        };
        for inst in &img.instances {
            let node = local[&inst.name.name];
            for m in &inst.mounts {
                let t = target(m)?;
                let whose = format!("`{}`'s view", inst.name.name);
                let decl = mount_decl(&m.at, t, m.read_only)?;
                push_mount(&mut self.nodes[node].mounts, decl, m.at.pos, &whose)?;
                self.nodes[node].written.push(m.at.pos);
            }
            for b in &inst.binds {
                let at = clean_mount_point(&b.at)?;
                let path = ns::clean(&b.path.path).map_err(|_| anyhow!("{}: `{}` is not a clean absolute path", b.path.pos, b.path.path))?;
                self.pending.push(Pending { node, export: false, at, from: image, path, read_only: b.read_only, pos: b.at.pos });
            }
            if let TypeRef::Builtin(b) = &self.nodes[node].ty
                && let Some(at) = inst.mounts.first().map(|m| &m.at).or(inst.binds.first().map(|b| &b.at))
            {
                bail!("{}: {b} has no view to mount into; it only serves", at.pos);
            }
            let is_packed = matches!(&self.nodes[node].ty, TypeRef::Builtin(n) if n == PACKED);
            if !is_packed && let Some(inc) = inst.includes.first() {
                bail!("{}: `include` packs files into a {PACKED} instance, and `{}` is a {}", inc.pos, inst.name.name, inst.type_name.name);
            }
            match self.nodes[node].ty {
                TypeRef::Image(child) => {
                    let given = self.slot_values(unit, slots, child, inst)?;
                    let prefix = format!("{prefix}{}.", inst.name.name);
                    self.instantiate(child, &prefix, Some(node), &given, depth + 1)?;
                }
                _ => {
                    if let Some((v, _)) = inst.values.first() {
                        bail!(err_at(v, format!("`{}` is a {}, which has no slots; only an image's instance takes values", inst.name.name, inst.type_name.name)));
                    }
                }
            }
        }
        for m in &img.mounts {
            let decl = mount_decl(&m.at, target(m)?, m.read_only)?;
            match image {
                Some(i) => {
                    push_mount(&mut self.nodes[i].mounts, decl, m.at.pos, "the image's view")?;
                    self.nodes[i].written.push(m.at.pos);
                }
                None => push_mount(&mut self.view, decl, m.at.pos, "the image's view")?,
            }
        }
        if let Some(i) = image {
            for e in &img.exports {
                let path = ns::clean(&e.path.path).map_err(|_| anyhow!("{}: `{}` is not a clean absolute path", e.path.pos, e.path.path))?;
                let at = clean_mount_point(e.at.as_ref().unwrap_or(&e.path))?;
                self.pending.push(Pending { node: i, export: true, at, from: Some(i), path, read_only: e.read_only, pos: e.path.pos });
            }
        }
        for r in &img.requests {
            self.requests.push(Request { image, unit, request: r.clone(), slots: slots.clone() });
        }
        Ok(())
    }

    /// The root image's needs, which the runtime fills with the real thing:
    /// each names one of its traits, and gets an instance named for it.
    fn root_needs(&mut self, unit: usize) -> Result<()> {
        for n in self.units[unit].image.header.needs.clone() {
            let named = match n.entries.as_slice() {
                [e] if e.path.path == "/" => match e.items.as_slice() {
                    [iface::Item::Named(id)] => self.units[unit].scope.traits.get(&id.name).map(|t| (t.name.clone(), id.clone())),
                    _ => None,
                },
                _ => None,
            };
            let Some((name, id)) = named.filter(|(name, _)| NATIVE.contains(&name.as_str())) else {
                bail!(
                    "{}: the image being run has its needs filled by the runtime, so each names one of its traits: {}",
                    n.at.pos,
                    NATIVE.join(", ")
                );
            };
            if iface::env().traits.get(&name) != self.units[unit].scope.traits.get(&id.name) {
                bail!(err_at(&id, format!("`{}` isn't the runtime's {name}; import it from env", id.name)));
            }
            let at = clean_mount_point(&n.at)?;
            let decl = MountDecl { at: at.clone(), target: self.nodes.len(), path: "/".into(), read_only: false };
            push_mount(&mut self.view, decl, n.at.pos, "the image's view")?;
            let ty = TypeRef::Builtin(name);
            self.nodes.push(Node { name: at, unit, ty, decl: None, mounts: vec![], written: vec![], exports: vec![], config: None });
        }
        Ok(())
    }

    /// The slots of `child`, an image instantiated as `inst` in `unit`,
    /// whose own slots are `slots`: each given in the instance block, or
    /// else its default.
    fn slot_values(&mut self, unit: usize, slots: &BTreeMap<String, Given>, child: usize, inst: &image_definition::Instance) -> Result<BTreeMap<String, Given>> {
        let requires = self.units[child].image.requires.clone();
        let mut out = BTreeMap::new();
        for (name, value) in &inst.values {
            let Some(req) = requires.iter().find(|r| r.name.name == name.name) else {
                bail!(err_at(name, format!("`{}` has no slot `{}`", inst.type_name.name, name.name)));
            };
            if out.contains_key(&name.name) {
                bail!(err_at(name, format!("slot `{}` is given twice", name.name)));
            }
            let what = format!("`{}`'s slot `{}`", inst.name.name, name.name);
            let given = match value {
                Literal::Name(id) => slots.get(&id.name).cloned().ok_or_else(|| err_at(id, format!("unknown slot `{}`", id.name)))?,
                literal => Given::Literal { literal: literal.clone(), unit, ty: content_type(&req.ty)?, what },
            };
            out.insert(name.name.clone(), given);
        }
        for req in &requires {
            let name = &req.name.name;
            let ty = content_type(&req.ty)?;
            if !out.contains_key(name) {
                let Some(default) = &req.default else {
                    bail!(err_at(&inst.name, format!("`{}` needs a value for its slot `{name}`: `{name} = ...` in its block", inst.name.name)));
                };
                let what = format!("`{}`'s slot `{name}`, as it defaults", inst.name.name);
                out.insert(name.clone(), Given::Literal { literal: default.clone(), unit: child, ty: ty.clone(), what });
            }
            if let Given::Arg(Arg::Slot(i)) = &out[name]
                && !payload::fits(&ty, &self.slots[*i].ty)
            {
                bail!(err_at(&inst.name, format!("`{}`'s slot `{name}` is a {ty}, and isn't given one", inst.name.name)));
            }
        }
        Ok(out)
    }

    /// What a slot is given, now that every module's schemas are known.
    fn given(&mut self, g: &Given) -> Result<Arg> {
        match g {
            Given::Arg(a) => Ok(a.clone()),
            Given::Literal { literal, unit, ty, what } => {
                let arg = self.literal(*unit, literal, Some(ty)).with_context(|| what.clone())?;
                if !payload::fits(ty, arg_type(&self.slots, &arg)) {
                    bail!("{what} is a {ty}, and isn't given one");
                }
                Ok(arg)
            }
        }
    }

    /// The payload `literal`, written in `unit`, carries; `want` is the
    /// content type of where it goes, if it is a slot's.
    fn literal(&mut self, unit: usize, literal: &Literal, want: Option<&str>) -> Result<Arg> {
        let ty = |t: &Option<Ident>, default: &str| -> Result<String> {
            Ok(match t {
                Some(t) => content_type(t)?,
                None => want.unwrap_or(default).to_string(),
            })
        };
        Ok(Arg::Payload(match literal {
            Literal::Text { ty: t, text, .. } => Payload::new(ty(t, payload::TEXT)?, text.as_bytes()),
            Literal::Tree { ty: None, pos, .. } if want.is_none() => {
                let example = self.loader.formats.iter().next().map(|f| f.content_type()).unwrap_or("application/bson");
                bail!("{pos}: say what this payload is: `<content type> {{ ... }}`, such as `{example} {{ ... }}`")
            }
            Literal::Tree { ty: t, value, pos } => {
                let ty = ty(t, "")?;
                let bytes = self.loader.formats.encode(&ty, &self.messages, value).map_err(|e| anyhow!("{pos}: {e}"))?;
                Payload::new(ty, bytes)
            }
            Literal::File { ty: t, path, pos } => {
                let file = iface::join(&self.units[unit].dir, path);
                let bytes = self.loader.read(&file).with_context(|| format!("{pos}: {} from {path:?}", t.name))?;
                Payload::new(content_type(t)?, bytes)
            }
            Literal::Name(id) => bail!(err_at(id, format!("unknown slot `{}`", id.name))),
        }))
    }

    /// What the instance type `ty` names in `unit`: an object it declares or
    /// imports, one of the runtime's objects in memory, or an image it
    /// imports. A module's interface is the header inside it, which must be
    /// the one it was built from.
    fn instance_type(&mut self, unit: usize, ty: &Ident) -> Result<TypeRef> {
        let image = self.units[unit].image.header.imports.iter().find(|i| i.default.as_ref().is_some_and(|d| d.name == ty.name));
        if let Some(imp) = image {
            let image_definition::Source::Path(p) = &imp.from else { unreachable!("env has no image") };
            let path = iface::join(&self.units[unit].dir, p);
            let rel = iface::join(&self.units[unit].rel, &Path::new(p).parent().unwrap_or(Path::new("")).to_string_lossy());
            return Ok(TypeRef::Image(self.unit(&path, &rel)?));
        }
        let scope = &self.units[unit].scope;
        if let Some(o) = scope.object_named(&ty.name) {
            let Some(origin) = o.origin.clone() else { return Ok(TypeRef::Builtin(o.name.clone())) };
            let o = o.clone();
            let module = if origin.extension().is_some_and(|e| e == "wasm") { origin.clone() } else { origin.with_extension("wasm") };
            if self.loader.read(&module).is_err() {
                bail!("cannot read {} (did you compile {}.ts?)", module.display(), o.name);
            }
            let header = self.loader.header(&module)?;
            iface::for_module(&header, &o.name).with_context(|| format!("{} has the wrong header", module.display()))?;
            let object = header.object(&o.name).cloned().expect("for_module found it");
            if !object.same_shape(&o) {
                bail!("{} is stale: the header inside it isn't {}'s; rebuild it", module.display(), origin.display());
            }
            let rel = module.strip_prefix(&self.dir).unwrap_or(&module).to_string_lossy().into_owned();
            if let Some(prev) = self.modules.insert(object.name.clone(), rel.clone())
                && prev != rel
            {
                bail!("`{}` would come from both {prev} and {rel}; an image can't use two modules of one name", object.name);
            }
            let formats = self.loader.formats.clone();
            for f in formats.iter() {
                let schemas = self.loader.schemas(f, &module)?;
                if origin.extension().is_some_and(|e| e == "oth") && !f.same(&self.loader.schemas(f, &origin)?, &schemas) {
                    bail!(
                        "{} is stale: the types inside it aren't those of {}'s `{}` schemas; rebuild it",
                        module.display(),
                        origin.display(),
                        f.content_type()
                    );
                }
                let ty = f.content_type().to_string();
                let from = self.message_from.entry(ty.clone()).or_default();
                f.merge(self.messages.entry(ty).or_default(), from, &schemas, &object.name)?;
            }
            return Ok(TypeRef::Wasm { object, module: rel });
        }
        if let Some(t) = scope.traits.get(&ty.name) {
            if NATIVE.contains(&t.name.as_str()) && iface::env().traits.get(&t.name) == Some(t) {
                bail!(err_at(
                    ty,
                    format!(
                        "no image declares the runtime's {}: the image being run gets one by needing it, e.g. `needs /dev/cons Console`, \
                         and gives it to what it instantiates",
                        t.name
                    )
                ));
            }
            let runs = IN_MEMORY.join(", ");
            bail!(err_at(ty, format!("`{}` is a shape, not an object; of the runtime's, an image can declare {runs}", ty.name)));
        }
        if let Some((_, now)) = RENAMED.iter().find(|(old, _)| *old == ty.name) {
            bail!(err_at(ty, format!("the runtime's objects have new names: `{now}`, imported from env")));
        }
        if IN_MEMORY.contains(&ty.name.as_str()) {
            bail!(err_at(ty, format!("`{}` is the runtime's; import it first: `import {{ {} }} from env`", ty.name, ty.name)));
        }
        bail!(err_at(ty, format!("unknown object `{}`; is it imported?", ty.name)))
    }

    fn is_image(&self, node: usize) -> bool {
        node < self.nodes.len() && matches!(self.nodes[node].ty, TypeRef::Image(_))
    }

    /// A namespace with every view built so far, and every export, for
    /// working out where paths lead.
    fn namespace(&self) -> Namespace {
        let n = self.nodes.len();
        let mut ns = Namespace::new(n + 1);
        for (i, node) in self.nodes.iter().enumerate() {
            for m in &node.mounts {
                ns.mount_resolved(i, &m.at, m.resolved());
            }
            for e in &node.exports {
                ns.expose_resolved(i, &e.at, e.resolved());
            }
        }
        for m in &self.view {
            ns.mount_resolved(n, &m.at, m.resolved());
        }
        ns
    }

    /// Works out every bind and export. Each is resolved in its image's
    /// view as soon as it leads to an object: an export can be of what
    /// another image exports, or of something bound, so they are taken in
    /// whatever order they can be.
    fn resolve_pending(&mut self) -> Result<()> {
        let n = self.nodes.len();
        // An image's needs for one mount point in particular are filled by
        // its instance's block, or nothing it binds from there can resolve.
        for (i, node) in self.nodes.iter().enumerate() {
            let TypeRef::Image(u) = node.ty else { continue };
            for need in self.units[u].scope.needs.iter().filter(|n| n.at.is_literal()) {
                let filled = node.mounts.iter().any(|m| need.at.matches(&m.at))
                    || self.pending.iter().any(|p| p.node == i && !p.export && need.at.matches(&p.at));
                if !filled {
                    let pos = node.decl.as_ref().map(|d| d.name.pos).unwrap_or(Pos { line: 0, col: 0 });
                    bail!("{pos}: `{}` needs something mounted at `{}`", node.name, need.at);
                }
            }
        }
        let mut pending = std::mem::take(&mut self.pending);
        let mut ns = self.namespace();
        while !pending.is_empty() {
            let before = pending.len();
            let mut left = vec![];
            for p in pending {
                match ns.resolve(p.from.unwrap_or(n), &p.path) {
                    Ok(mut r) if !self.is_image(r.inst) => {
                        r.read_only |= p.read_only;
                        let decl = MountDecl { at: p.at.clone(), target: r.inst, path: r.path.clone(), read_only: r.read_only };
                        if p.export {
                            ns.expose_resolved(p.node, &p.at, r);
                            push_mount(&mut self.nodes[p.node].exports, decl, p.pos, "what the image exports")?;
                        } else {
                            ns.mount_resolved(p.node, &p.at, r);
                            let whose = format!("`{}`'s view", self.nodes[p.node].name);
                            push_mount(&mut self.nodes[p.node].mounts, decl, p.pos, &whose)?;
                            self.nodes[p.node].written.push(p.pos);
                        }
                    }
                    _ => left.push(p),
                }
            }
            if left.len() == before {
                let p = &left[0];
                let whose = match p.from {
                    Some(i) => format!("`{}`'s", self.nodes[i].name),
                    None => "the image's".into(),
                };
                bail!("{}: nothing is at `{}` in {whose} view", p.pos, p.path);
            }
            pending = left;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Program> {
        let n = self.nodes.len();
        for node in &mut self.nodes {
            let mut both: Vec<(Pos, MountDecl)> = node.written.drain(..).zip(node.mounts.drain(..)).collect();
            both.sort_by_key(|(p, _)| (p.line, p.col));
            node.mounts = both.into_iter().map(|(_, m)| m).collect();
        }
        let ns = self.namespace();
        for slot in &self.slots {
            if let Some(SlotValue::Tree(v)) = &slot.default
                && let Some(f) = self.loader.formats.for_type(&slot.ty)
            {
                let schemas = self.messages.get(f.content_type()).cloned().unwrap_or_default();
                f.check(&schemas, v).map_err(|e| anyhow!("slot `{}`'s default: {e}", slot.name))?;
            }
        }

        // Every interface: the objects', then each image's, which serves
        // what its exports lead to, always objects.
        let mut interfaces: BTreeMap<String, Object> = BTreeMap::new();
        let mut type_names = vec![];
        for node in &self.nodes {
            let o = match &node.ty {
                TypeRef::Builtin(name) => builtin(name).expect("only the runtime's are Builtin"),
                TypeRef::Wasm { object, .. } => object.clone(),
                TypeRef::Image(u) => Object {
                    name: format!("image {}", node.name),
                    doc: String::new(),
                    traits: vec![],
                    guide: None,
                    needs: self.units[*u].scope.needs.clone(),
                    serves: vec![],
                    origin: None,
                },
            };
            type_names.push(o.name.clone());
            interfaces.insert(o.name.clone(), o);
        }
        for (i, node) in self.nodes.iter().enumerate() {
            if matches!(node.ty, TypeRef::Image(_)) {
                let serves = node.exports.iter().flat_map(|e| served(&interfaces, &type_names, e)).collect();
                interfaces.get_mut(&type_names[i]).expect("just made").serves = serves;
            }
        }

        // Every view against what its owner needs.
        for (i, node) in self.nodes.iter().enumerate() {
            let obj = &interfaces[&type_names[i]];
            let at = node.decl.as_ref().map(|d| d.name.pos).unwrap_or(Pos { line: 0, col: 0 });
            self.check_view(obj, &node.name, at, &node.mounts, &interfaces, &type_names)?;
        }

        // Every startup request, as the operator, against what it reaches.
        // The root's needs are configured by its writes of their roots.
        let mut startup = vec![];
        let mut configs: Vec<Vec<(Pos, Arg)>> = vec![vec![]; n];
        for r in std::mem::take(&mut self.requests) {
            let req = &r.request;
            let full = ns::clean(&req.path.path).map_err(|_| anyhow!("{}: `{}` is not a clean absolute path", req.path.pos, req.path.path))?;
            let view = r.image.unwrap_or(n);
            let to = ns.resolve(view, &full).map_err(|_| anyhow!("{}: nothing is mounted at `{full}` in the image's view", req.path.pos))?;
            if to.read_only && !req.verb.is_safe() {
                bail!("{}: `{full}` is read-only in the image's view, so it can't take `{}`", req.verb_pos, req.verb);
            }
            let body = match &req.body {
                Some(Literal::Name(id)) => {
                    let g = r.slots.get(&id.name).ok_or_else(|| err_at(id, format!("unknown slot `{}`", id.name)))?;
                    Some(self.given(g)?)
                }
                Some(b) => Some(self.literal(r.unit, b, None)?),
                None => None,
            };
            let configured = matches!(&self.nodes[to.inst].ty, TypeRef::Builtin(b) if CONFIGURED.contains(&b.as_str()));
            if configured && req.verb == Verb::Write && to.path == "/" {
                configs[to.inst].push((req.verb_pos, body.ok_or_else(|| anyhow!("{}: a config needs text", req.verb_pos))?));
                continue;
            }
            let name = self.nodes[to.inst].name.clone();
            check_request(req, &to.path, &name, &interfaces[&type_names[to.inst]], body.as_ref(), &self.slots)?;
            startup.push(Startup { image: r.image, verb: req.verb, path: full, body });
        }
        for (i, node) in self.nodes.iter_mut().enumerate() {
            let TypeRef::Builtin(b) = &node.ty else { continue };
            let Some(what) = config_what(b) else { continue };
            let needs = format!("{} needs exactly one config: `write` its root with {what}", node.name);
            let [(pos, arg)] = configs[i].as_slice() else { bail!(needs) };
            if !payload::fits("text/*", arg_type(&self.slots, arg)) {
                bail!("{pos}: {needs}");
            }
            node.config = Some(arg.clone());
        }

        let instances = self
            .nodes
            .into_iter()
            .zip(type_names)
            .map(|(node, type_name)| {
                let kind = match node.ty {
                    TypeRef::Builtin(b) => {
                        let includes = node.decl.as_ref().map(|d| d.includes.clone()).unwrap_or_default();
                        builtin_kind(&b, &includes)?
                    }
                    TypeRef::Wasm { object, module, .. } => Kind::Wasm { type_name: object.name, module },
                    TypeRef::Image(_) => Kind::Image { exports: node.exports },
                };
                let dir = self.units[node.unit].rel.to_string_lossy().into_owned();
                Ok(Instance { name: node.name, type_name, kind, dir, config: node.config, mounts: node.mounts })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Program { messages: self.messages, interfaces, slots: self.slots, instances, view: self.view, startup, sources: vec![] })
    }

    /// `mounts`, the view of the instance `name`, against what `obj` needs
    /// there, and each need for one mount point in particular against them.
    fn check_view(
        &self,
        obj: &Object,
        name: &str,
        pos: Pos,
        mounts: &[MountDecl],
        interfaces: &BTreeMap<String, Object>,
        type_names: &[String],
    ) -> Result<()> {
        for m in mounts {
            let Some(need) = obj.need(&m.at) else { continue };
            let target = &self.nodes[m.target].name;
            let Some(full) = interfaces.get(&type_names[m.target]) else { continue };
            let mut served = full.clone();
            served.serves = below(full, &m.path, "/");
            for e in &need.entries {
                for r in &e.requests {
                    if m.read_only && !r.verb.is_safe() {
                        bail!("`{name}` needs `{} {r}` at `{}`, but `{target}` is mounted there read-only", e.path, m.at);
                    }
                    if !serves_need(&served, need, e, r) {
                        let shown: Vec<String> =
                            served.serves.iter().filter(|s| s.path.overlaps(&e.path)).map(|s| format!("`{s}`")).collect();
                        let has = if shown.is_empty() { "nothing there".to_string() } else { shown.join(", ") };
                        bail!("`{name}` needs `{} {r}` at `{}`, and `{target}` serves {has}", e.path, m.at);
                    }
                }
            }
        }
        for need in obj.needs.iter().filter(|n| n.at.is_literal()) {
            if !mounts.iter().any(|m| need.at.matches(&m.at)) {
                bail!("{pos}: `{name}` needs something mounted at `{}`", need.at);
            }
        }
        Ok(())
    }
}

/// What an image's instance serves through export `e`: what its target
/// serves below the exported path, put where the export puts it. Its
/// target is an object, whose interface is already known.
fn served(interfaces: &BTreeMap<String, Object>, type_names: &[String], e: &MountDecl) -> Vec<iface::Entry> {
    let Some(target) = type_names.get(e.target).and_then(|t| interfaces.get(t)) else { return vec![] };
    below(target, &e.path, &e.at)
}

/// What `obj` serves below `path`, put under the mount point `at`.
fn below(obj: &Object, path: &str, at: &str) -> Vec<iface::Entry> {
    obj.serves
        .iter()
        .filter_map(|e| {
            let rel = e.path.below(path)?;
            Some(iface::Entry { path: Pattern::under(at, &rel), doc: e.doc.clone(), requests: e.requests.clone() })
        })
        .collect()
}

fn clean_mount_point(p: &image_definition::Path) -> Result<String> {
    ns::mount_point(&p.path).map_err(|_| anyhow!("{}: `{}` is not a clean absolute path", p.pos, p.path))
}

fn mount_decl(at: &image_definition::Path, target: usize, read_only: bool) -> Result<MountDecl> {
    Ok(MountDecl { at: clean_mount_point(at)?, target, path: "/".into(), read_only })
}

/// Adds `decl` to `mounts`, which `whose` names in errors.
fn push_mount(mounts: &mut Vec<MountDecl>, decl: MountDecl, pos: Pos, whose: &str) -> Result<()> {
    if mounts.iter().any(|prev| prev.at == decl.at) {
        bail!("{pos}: `{}` is already mounted in {whose}", decl.at);
    }
    mounts.push(decl);
    Ok(())
}

/// A startup request of `path` in instance `name`'s served tree, against
/// what `obj` serves there.
fn check_request(r: &image_definition::Request, path: &str, name: &str, obj: &Object, body: Option<&Arg>, slots: &[Slot]) -> Result<()> {
    let body_type = body.map(|b| arg_type(slots, b));
    let Some(entry) = obj.serving(path) else {
        bail!("{}: `{name}` serves nothing at `{path}`", r.verb_pos);
    };
    let given = match body_type {
        Some(t) => format!("{} {t}", r.verb),
        None => r.verb.to_string(),
    };
    match entry.takes(r.verb, body_type) {
        Ok(_) => Ok(()),
        Err(Status::NotSupported) => bail!("{}: `{name}` doesn't take `{}` at `{path}`; it serves `{entry}`", r.verb_pos, r.verb),
        Err(_) => bail!("{}: `{name}` doesn't take `{given}` at `{path}`; it serves `{entry}`", r.verb_pos),
    }
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

/// The interface of the runtime's object `name`, if it is one: one of the
/// traits it implements on the host, as an object serving the trait's tree,
/// or one of its objects in memory.
fn builtin(name: &str) -> Option<Object> {
    let env = iface::env();
    if IN_MEMORY.contains(&name) {
        return env.object(name).cloned();
    }
    env.traits.get(name).filter(|_| NATIVE.contains(&name)).map(Trait::as_object)
}

/// What the runtime's object `name` is configured with, for errors; `None`
/// if it takes no config.
fn config_what(name: &str) -> Option<&'static str> {
    match name {
        LISTENER => Some("the address to listen on, e.g. `\"127.0.0.1:8080\"`"),
        FILES => Some("the directory to serve, e.g. `\"public\"`"),
        _ => None,
    }
}

/// How the runtime implements its object `name`. A `PackedFiles` serves its
/// `includes`.
fn builtin_kind(name: &str, includes: &[image_definition::Include]) -> Result<Kind> {
    Ok(match name {
        CONSOLE => Kind::Console,
        LISTENER => Kind::Tcp,
        FILES => Kind::Files,
        MEMORY_CONSOLE => Kind::MemoryConsole,
        MEMORY_LISTENER => Kind::MemoryListener,
        PACKED => {
            let mut decls = vec![];
            for inc in includes {
                let at = match &inc.at {
                    Some(p) => Some(clean_mount_point(p)?),
                    None => None,
                };
                decls.push(IncludeDecl { source: inc.source.clone(), at });
            }
            Kind::Packed { includes: decls }
        }
        other => bail!("the runtime doesn't implement {other}"),
    })
}

/// The content type of what `arg` carries.
fn arg_type<'a>(slots: &'a [Slot], arg: &'a Arg) -> &'a str {
    match arg {
        Arg::Payload(p) => &p.ty,
        Arg::Slot(i) => &slots[*i].ty,
    }
}

/// The content type an image writes, which must be well-formed.
fn content_type(id: &Ident) -> Result<String> {
    if !payload::valid(&id.name) {
        bail!(err_at(id, format!("`{}` isn't a well-formed content type", id.name)));
    }
    Ok(id.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use format_bson::{Bson, wire};
    use syntax::image_definition::Value;

    fn formats() -> Formats {
        Formats::new(vec![Box::new(Bson)])
    }

    /// The message types every scripted module declares, beside its own.
    const MESSAGES: &str = r#"
        message Greeting { text: string }
        message Config { name: string, count: u8, ratio: f64, on: bool }
        message Ping {}
        message Blob { data: bytes }
    "#;

    /// `src` with every declaration exported.
    fn exported(src: &str) -> String {
        let mut out = src.to_string();
        for kw in ["protocol ", "trait ", "object "] {
            out = out.replace(&format!("export {kw}"), kw).replace(kw, &format!("export {kw}"));
        }
        out
    }

    /// `src` taken apart: its `message` declarations, which go in its
    /// schemas block, and the header that is left.
    fn split(src: &str) -> (String, String) {
        let (mut header, mut messages) = (String::new(), String::new());
        let mut rest = src;
        while let Some(at) = rest.find("message ") {
            header.push_str(&rest[..at]);
            let end = at + rest[at..].find('}').unwrap() + 1;
            messages.push_str(&rest[at..end]);
            messages.push('\n');
            rest = &rest[end..];
        }
        header.push_str(rest);
        (header, messages)
    }

    /// A module whose header is `src`, which can only import from env, and
    /// whose message types are `messages`.
    fn module(src: &str, messages: &str) -> Vec<u8> {
        let mut loader = Loader::new(formats());
        let path = Path::new("/module/M.oth");
        loader.files.insert(path.into(), with_schemas(src, messages).into_bytes());
        let header = loader.header(path).unwrap();
        let schemas = loader.schemas(&Bson, path).unwrap();
        let wasm = iface::embed(&wat::parse_str("(module)").unwrap(), &header).unwrap();
        iface::embed_schemas(&wasm, &Bson, &schemas).unwrap()
    }

    /// `header` with `messages` in its schemas block.
    fn with_schemas(header: &str, messages: &str) -> String {
        format!("{header}\nexport schemas application/bson {{\n{messages}\n}}\n")
    }

    /// Puts module `ty`, with header `src`, at /check: its header and
    /// schemas, and the module, which declares MESSAGES too.
    fn put_module(loader: &mut Loader, dir: &Path, ty: &str, src: &str) {
        let (header, own) = split(src);
        let header = exported(&header);
        let messages = format!("{MESSAGES}\n{own}");
        loader.files.entry(dir.join(format!("{ty}.oth"))).or_insert_with(|| with_schemas(&header, &messages).into_bytes());
        if ty != "Ghost" {
            loader.files.entry(dir.join(format!("{ty}.wasm"))).or_insert_with(|| module(&header, &messages));
        }
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

    /// Checks `img` as /check/test.oth, beside `files`. Any instance type it
    /// doesn't import is imported for it at the top: from env if it is the
    /// runtime's, and otherwise from ./<Type>.oth, which has its module
    /// beside it, with the header in `modules` or else a permissive one, and
    /// MESSAGES among its message types. A `Ghost` has no module.
    fn check_all(img: &str, modules: &[(&str, &str)], files: &[(&str, &str)]) -> Result<Program> {
        let dir = PathBuf::from("/check");
        let mut loader = Loader::new(formats());
        for (name, src) in files {
            loader.files.insert(dir.join(name), src.as_bytes().to_vec());
        }
        let parsed = image_definition::parse(img)?;
        let mut imported: Vec<String> = parsed
            .header
            .imports
            .iter()
            .flat_map(|i| i.names.iter().map(|n| n.local.name.clone()).chain(i.default.iter().map(|d| d.name.clone())))
            .collect();
        let (mut prelude, mut from_env) = (String::new(), vec![]);
        let mut names: Vec<String> = parsed.instances.iter().map(|i| i.type_name.name.clone()).collect();
        for n in &parsed.header.needs {
            if let [e] = n.entries.as_slice()
                && let [iface::Item::Named(id)] = e.items.as_slice()
            {
                names.push(id.name.clone());
            }
        }
        for ty in names {
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
            let src = modules.iter().find(|(n, _)| *n == ty).map(|(_, s)| s.to_string()).unwrap_or_else(|| permissive(&ty));
            put_module(&mut loader, &dir, &ty, &src);
            prelude.push_str(&format!("import {{ {ty} }} from \"./{ty}.oth\" "));
        }
        if !from_env.is_empty() {
            prelude.push_str(&format!("import {{ {} }} from env ", from_env.join(", ")));
        }
        for (ty, src) in modules {
            put_module(&mut loader, &dir, ty, src);
        }
        loader.files.insert(dir.join("test.oth"), format!("{prelude}{img}").into_bytes());
        load_with(&dir.join("test.oth"), &mut loader)
    }

    fn check_with(img: &str, modules: &[(&str, &str)]) -> Result<Program> {
        check_all(img, modules, &[])
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

    fn names(p: &Program) -> Vec<&str> {
        p.instances.iter().map(|i| i.name.as_str()).collect()
    }

    const HEADER: &str = r#"
        instance cons : MemoryConsole
    "#;

    /// The message in a BSON payload.
    fn message(arg: &Arg) -> wire::Message {
        wire::Message::decode(&arg.payload().unwrap().bytes).unwrap()
    }

    fn mount(at: &str, target: usize, read_only: bool) -> MountDecl {
        MountDecl { at: at.into(), target, path: "/".into(), read_only }
    }

    #[test]
    fn accepts_hello_world() {
        let p = check_src("needs /dev/cons Console  instance echo : Echo { bind /dev/cons /dev/cons }").unwrap();
        assert_eq!(names(&p), ["/dev/cons", "echo"], "the root's needs come first, named for their mount points");
        assert_eq!(p.instances[0].kind, Kind::Console);
        assert_eq!(p.instances[1].kind, Kind::Wasm { type_name: "Echo".into(), module: "Echo.wasm".into() });
        assert_eq!(p.instances[1].mounts, [mount("/dev/cons", 0, false)]);
        assert_eq!(p.view, [mount("/dev/cons", 0, false)]);
    }

    #[test]
    fn mounts_are_checked() {
        assert!(err(&format!("{HEADER} instance a : A {{ mount /g ghost }}")).contains("unknown instance `ghost`"));
        let e = err(&format!("{HEADER} instance a : A {{ mount /x/../y cons }}"));
        assert!(e.contains("`/x/../y` is not a clean absolute path"), "{e}");
        let e = err(&format!("{HEADER} instance a : A {{ mount /c cons mount /c/ cons }}"));
        assert!(e.contains("`/c` is already mounted"), "{e}");
        let e = err(&format!("{HEADER} instance m : MemoryConsole {{ mount /c cons }}"));
        assert!(e.contains("MemoryConsole has no view to mount into; it only serves"), "{e}");
        // Forward references are fine: views are built once every instance exists.
        let p = check_src(&format!("{HEADER} instance a : A {{ mount /b b readonly }} instance b : B")).unwrap();
        assert_eq!(p.instances[1].mounts[0], mount("/b", 2, true));
        let e = err(&format!("{HEADER} instance a : A {{ bind /x /nowhere }}"));
        assert!(e.contains("nothing is at `/nowhere` in the image's view"), "{e}");
    }

    #[test]
    fn binds_give_what_a_path_means_in_the_images_view() {
        let p = check_src(&format!("{HEADER} instance b : B  mount /b b  instance a : A {{ bind /x /b/sub readonly  mount /c cons }}")).unwrap();
        assert_eq!(p.instances[2].mounts, [MountDecl { at: "/x".into(), target: 1, path: "/sub".into(), read_only: true }, mount("/c", 0, false)]);
        let d = describe(&p, &|_| None, &|_| false);
        assert!(d.contains("a     sees /x → b/sub ro, /c → cons\n"), "{d}");
    }

    #[test]
    fn startup_requests_are_typed() {
        let inst = |req: &str| format!("{HEADER} instance e : Echo mount /e e {req}");
        let p = check_src(&inst(
            r#"write /e/config application/bson { $type: "Config", name: "x", count: 3, ratio: 2, on: true }
               create /e application/bson { $type: "Ping" }  remove /e/a//b/"#,
        ))
        .unwrap();
        let s = &p.startup;
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].image, s[0].verb, s[0].path.as_str()), (None, Verb::Write, "/e/config"));
        let body = message(s[0].body.as_ref().unwrap());
        assert_eq!(body.fields.get_i32("count").unwrap(), 3);
        assert_eq!(body.fields.get_f64("ratio").unwrap(), 2.0);
        assert_eq!(message(s[1].body.as_ref().unwrap()).type_name, "Ping");
        assert_eq!((s[2].verb, s[2].path.as_str(), &s[2].body), (Verb::Remove, "/e/a/b/", &None));

        let bson = |fields: &str| inst(&format!("write /e/c application/bson {{ {fields} }}"));
        assert!(err(&bson(r#"$type: "Config", name: "x""#)).contains("`Config` is missing field `count`"));
        assert!(err(&bson(r#"$type: "Config", name: 1, count: 1, ratio: 1, on: true"#)).contains("`Config.name` is string, but was given 1"));
        assert!(err(&bson(r#"$type: "Config", name: "", count: 256, ratio: 1, on: true"#)).contains("does not fit in u8"));
        assert!(err(&bson(r#"$type: "Ping", x: 1"#)).contains("`Ping` has no field `x`"));
        assert!(err(&bson(r#"$type: "Missing""#)).contains("no module in the image declares a message type `Missing`"));
        assert!(err(&bson(r#"$type: "Blob", data: 1"#)).contains("is bytes"));
        assert!(err(&inst("write /e/c { x: 1 }")).contains("say what this payload is: `<content type> { ... }`"));
        assert!(err(&inst("write /e/c text/plain { x: 1 }")).contains("no format writes `text/plain`"));
        let e = err(&inst(r#"write /e/c Config { name: "x" }"#));
        assert!(e.contains("write the payload's content type, and the type inside it: `application/bson { $type: \"Config\", ... }`"), "{e}");
        assert!(err(&inst("read /e/../x")).contains("not a clean absolute path"));
        assert!(err(&inst("read /x")).contains("nothing is mounted at `/x` in the image's view"));
        assert!(err(&inst("read /")).contains("nothing is mounted at `/` in the image's view"));
        assert!(err(&format!("{HEADER} write /e \"x\" instance e : Echo")).contains("nothing is mounted at `/e`"));
    }

    #[test]
    fn a_name_is_imported_once_in_a_file() {
        let twice = "import { Console } from env import { Console } from env";
        assert!(format!("{:#}", check_src(twice).unwrap_err()).contains("`Console` is already imported"));
    }

    #[test]
    fn a_payload_can_be_a_file() {
        let img = format!(r#"{HEADER} instance e : Echo mount /e e write /e/page text/html from "./page.html""#);
        let p = check_all(&img, &[], &[("page.html", "<p>hi</p>")]).unwrap();
        assert_eq!(p.startup[0].body, Some(Arg::Payload(Payload::new("text/html", "<p>hi</p>"))));
        let e = format!("{:#}", check_all(&img, &[], &[]).unwrap_err());
        assert!(e.contains("text/html from \"./page.html\"") && e.contains("reading /check/page.html"), "{e}");
    }

    #[test]
    fn startup_requests_keep_the_order_they_are_written_in() {
        let p = check_src(&format!(r#"{HEADER} write /b "1" instance a : A mount /a a write /a "2" instance b : B mount /b b write /b/x "3""#))
            .unwrap();
        let order: Vec<&str> = p.startup.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(order, ["/b", "/a", "/b/x"]);
    }

    #[test]
    fn requests_resolve_in_the_images_view() {
        let p = check_src(&format!("{HEADER} mount / e  mount /e/out cons  instance e : Echo  write /x \"x\"  write /e/out \"hi\"")).unwrap();
        assert_eq!(p.view, [mount("/", 1, false), mount("/e/out", 0, false)]);
        let paths: Vec<&str> = p.startup.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["/x", "/e/out"], "kept as written, and resolved again when the image runs");

        let e = err(&format!("{HEADER} mount /e e readonly  instance e : Echo  write /e \"x\""));
        assert!(e.contains("`/e` is read-only in the image's view, so it can't take `write`"), "{e}");
        assert!(check_src(&format!("{HEADER} mount /e e readonly  instance e : Echo  read /e")).is_ok());
        let e = err(&format!("{HEADER} mount /c cons  mount /c/ cons"));
        assert!(e.contains("`/c` is already mounted in the image's view"), "{e}");
        let e = err(&format!("{HEADER} mount /g ghost"));
        assert!(e.contains("unknown instance `ghost`"), "{e}");
    }

    #[test]
    fn bytes_fields_take_string_literals_as_utf8() {
        let p = check_src(&format!(r#"{HEADER} instance e : Echo mount /e e write /e application/bson {{ $type: "Blob", data: "hé" }}"#)).unwrap();
        let body = message(p.startup[0].body.as_ref().unwrap());
        let data = body.fields.get_binary_generic("data").unwrap();
        assert_eq!(data, "hé".as_bytes());
    }

    #[test]
    fn the_root_needs_the_runtimes_objects_and_configures_them() {
        let p = check_src(r#"needs /net Listener  write /net "127.0.0.1:0"  needs /fs Files  write /fs "public""#).unwrap();
        assert_eq!(names(&p), ["/net", "/fs"]);
        assert_eq!(p.instances[0].kind, Kind::Tcp);
        assert_eq!(p.instances[0].config.as_ref().unwrap().text().unwrap(), "127.0.0.1:0");
        assert_eq!(p.instances[1].kind, Kind::Files);
        assert!(p.startup.is_empty(), "config isn't a request");

        let e = err("needs /net Listener");
        assert!(e.contains("/net needs exactly one config: `write` its root with the address to listen on, e.g. `\"127.0.0.1:8080\"`"), "{e}");
        let e = err(r#"needs /net Listener  write /net "a"  write /net "b""#);
        assert!(e.contains("exactly one"), "{e}");
        let e = err(&format!(r#"{HEADER} instance e : Echo  needs /net Listener  write /net application/bson {{ $type: "Ping" }}"#));
        assert!(e.contains("exactly one config"), "it takes text: {e}");
        let p = check_src(r#"needs /c Console  write /c "printed""#).unwrap();
        assert_eq!(p.startup.len(), 1, "a Console takes no config, so writing it prints");
        let e = err(r#"needs /net Listener  write /net "a"  write /net/x "b""#);
        assert!(e.contains("`/net` doesn't take `write` at `/x`"), "{e}");

        let e = err("needs /x FileTree");
        assert!(e.contains("each names one of its traits: Console, Listener, Files"), "{e}");
        let e = err("needs /x { / read }");
        assert!(e.contains("each names one of its traits"), "{e}");
        let e = err("instance c : Console");
        assert!(e.contains("no image declares the runtime's Console: the image being run gets one by needing it"), "{e}");
        let e = err("instance c : Environment.Stdin");
        assert!(e.contains("unknown object `Environment.Stdin`"), "{e}");
        let e = err("instance net : Environment.Tcp");
        assert!(e.contains("the runtime's objects have new names: `Listener`, imported from env"), "{e}");
        let e = err("instance t : FileTree");
        assert!(e.contains("`FileTree` is a shape, not an object; of the runtime's, an image can declare PackedFiles, MemoryConsole, MemoryListener"), "{e}");
    }

    #[test]
    fn the_configured_objects_take_text_at_their_root() {
        for name in CONFIGURED {
            assert!(config_what(name).is_some(), "{name}");
            let root = builtin(name).unwrap().serves.into_iter().find(|e| e.path.as_str() == "/").unwrap();
            assert!(root.takes(Verb::Write, Some("text/plain")).is_ok_and(|r| r.operator), "{name}: {root}");
        }
        assert_eq!(config_what(CONSOLE), None);
        assert_eq!(config_what(PACKED), None);
        for name in NATIVE.iter().chain(IN_MEMORY) {
            assert!(iface::env().traits.contains_key(*name) || iface::env().object(name).is_some(), "env declares {name}");
            assert!(builtin_kind(name, &[]).is_ok(), "the runtime implements {name}");
            assert!(builtin(name).is_some(), "{name}");
        }
    }

    #[test]
    fn packed_files_take_includes_and_nothing_else() {
        let p = check_src(
            r#"instance files : PackedFiles { include "public/" include "a.txt" at /b//c/ }
            instance site : Site { mount /files files }"#,
        )
        .unwrap();
        let want = [
            IncludeDecl { source: "public/".into(), at: None },
            IncludeDecl { source: "a.txt".into(), at: Some("/b/c".into()) },
        ];
        assert_eq!(p.instances[0].kind, Kind::Packed { includes: want.to_vec() });

        let e = err(r#"instance s : Site { include "x" }"#);
        assert!(e.contains("`include` packs files into a PackedFiles instance, and `s` is a Site"), "{e}");
        let e = err(r#"instance f : PackedFiles { include "x" at /../y }"#);
        assert!(e.contains("not a clean absolute path"), "{e}");
        let e = err(r#"instance f : PackedFiles mount /f f write /f ".""#);
        assert!(e.contains("`f` doesn't take `write` at `/`"), "it takes no config: {e}");
        let e = err(r#"instance f : Packed.Files"#);
        assert!(e.contains("the runtime's objects have new names: `PackedFiles`"), "{e}");
    }

    #[test]
    fn slots_stand_in_for_payloads() {
        let p = check_src(&format!(
            r#"{HEADER}
            require listen : text/plain
            require config : application/bson = {{ $type: "Config", name: "x", count: 3 }}
            needs /net Listener  write /net listen
            instance e : Echo mount /e e write /e/config config"#
        ))
        .unwrap();
        assert_eq!(p.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["listen", "config"]);
        let Some(SlotValue::Tree(Value::Object(fields))) = &p.slots[1].default else { panic!("{:?}", p.slots[1].default) };
        assert_eq!(fields.len(), 3, "defaults may be partial");
        assert_eq!(p.instances[0].config, Some(Arg::Slot(0)));
        assert_eq!(p.startup[0].body, Some(Arg::Slot(1)));

        let inst = |decls: &str| format!("{HEADER} {decls} instance e : Echo mount /e e");
        let e = err(&inst("require g : text/plain require g : text/html"));
        assert!(e.contains("slot `g` is declared twice"), "{e}");
        let e = err(&inst(r#"require g : application/bson = { $type: "Greeting", text: 1 }"#));
        assert!(e.contains("slot `g`'s default: `Greeting.text` is string"), "{e}");
        let e = err(&inst(r#"require g : application/bson = { $type: "Greeting", nope: 1 }"#));
        assert!(e.contains("`Greeting` has no field `nope`"), "{e}");
        let e = err(&inst("require g : Greeting"));
        assert!(e.contains("a slot is typed by a content type: `require g : application/bson = { $type: \"Greeting\", ... }`"), "{e}");
        let e = err(&inst(r#"require g : text/plain = text/html "x""#));
        assert!(e.contains("slot `g`'s default is text or a structured value, of its type"), "{e}");
        let e = err(&format!("{HEADER} require t : application/bson needs /net Listener write /net t"));
        assert!(e.contains("needs exactly one config"), "{e}");
    }

    #[test]
    fn types_are_imported_from_headers() {
        let greeter = "message Hello { prefix: string } object Greeter { serves { /greeting write Hello } }";
        let p = check_with(
            r#"instance g : Greeter mount /g g write /g/greeting application/bson { $type: "Hello", prefix: "hi" }"#,
            &[("Greeter", greeter)],
        )
        .unwrap();
        let types = Bson::messages(&p.messages[format_bson::BSON]).types;
        assert!(types.contains_key("Hello"), "a module's message types are the image's to write");
        assert_eq!(p.interfaces["Greeter"].serves[0].path.as_str(), "/greeting");
        let e = err(r#"needs /c Console  write /c application/bson { $type: "Hello" }"#);
        assert!(e.contains("no module in the image declares a message type `Hello`"), "only its modules': {e}");

        let e = err(r#"import { Ping } from "./messages.ts""#);
        assert!(e.contains("/check/messages.ts is neither a `.oth` file nor a module (`.wasm`)"), "{e}");
        let e = err("instance g : Ghost");
        assert!(e.contains("instance `g`") && e.contains("cannot read /check/Ghost.wasm (did you compile Ghost.ts?)"), "{e}");
        let e = err("instance g : Nobody.Here");
        assert!(e.contains("unknown object `Nobody.Here`; is it imported?"), "{e}");

        // Renamed, the object is still itself.
        let p = check_with(r#"import { Greeter as G } from "./Greeter.oth" instance g : G"#, &[("Greeter", greeter)]).unwrap();
        assert_eq!(p.instances[0].type_name, "Greeter");

        // A module must be the one its header and schemas were built into.
        let (header, messages) = split(greeter);
        let header = exported(&header);
        let stale = |oth: &str, schemas: &str| {
            let mut loader = Loader::new(formats());
            loader.files.insert("/s/Greeter.oth".into(), with_schemas(oth, schemas).into_bytes());
            loader.files.insert("/s/Greeter.wasm".into(), module(&header, &messages));
            loader.files.insert("/s/test.oth".into(), br#"import { Greeter } from "./Greeter.oth" instance g : Greeter"#.to_vec());
            format!("{:#}", load_with(Path::new("/s/test.oth"), &mut loader).unwrap_err())
        };
        let e = stale("export object Greeter { serves { /greeting write text/plain } }", &messages);
        assert!(e.contains("/s/Greeter.wasm is stale: the header inside it isn't /s/Greeter.oth's; rebuild it"), "{e}");
        let e = stale(&header, "message Hello { prefix: i32 }");
        assert!(e.contains("/s/Greeter.wasm is stale: the types inside it aren't those of /s/Greeter.oth's `application/bson` schemas; rebuild it"), "{e}");

        // Or it is imported from the module itself.
        let mut loader = Loader::new(formats());
        loader.files.insert("/m/Greeter.wasm".into(), module(&header, &messages));
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
        let greeter = "object G { serves { /greeting write Greeting  /hosts/{name} operator write text/plain, operator remove } }";
        let ok = r#"instance g : G mount /g g write /g/greeting application/bson { $type: "Greeting", text: "hi" } write /g/hosts/* "/www" remove /g/hosts/a"#;
        assert!(check_with(ok, &[("G", greeter)]).is_ok());
        let e = err_with("instance g : G mount /g g write /g/nothing", &[("G", greeter)]);
        assert!(e.contains("`g` serves nothing at `/nothing`"), "{e}");
        let e = err_with("instance g : G mount /g g read /g/greeting", &[("G", greeter)]);
        assert!(e.contains("`g` doesn't take `read` at `/greeting`; it serves `/greeting write Greeting`"), "{e}");
        let e = err_with("instance g : G mount /g g write /g/greeting", &[("G", greeter)]);
        assert!(e.contains("doesn't take `write` at `/greeting`"), "{e}");
        let only_text = "object G { serves { /greeting write text/plain } }";
        let e = err_with(r#"instance g : G mount /g g write /g/greeting application/bson { $type: "Greeting", text: "x" }"#, &[("G", only_text)]);
        assert!(e.contains("doesn't take `write application/bson` at `/greeting`"), "{e}");
        let texts = "object T { serves { /plain write text/plain  /any write text/* } }";
        let ok = r#"require t : text/plain = "x" instance t : T mount /t t write /t/plain "hi" write /t/any text/html "<p>" write /t/plain t"#;
        assert!(check_with(ok, &[("T", texts)]).is_ok(), "{:#}", check_with(ok, &[("T", texts)]).unwrap_err());
        let e = err_with(r#"instance t : T mount /t t write /t/plain text/html "<p>""#, &[("T", texts)]);
        assert!(e.contains("doesn't take `write text/html` at `/plain`"), "{e}");
        let e = err_with(r#"instance t : T mount /t t write /t/plain text/ "x""#, &[("T", texts)]);
        assert!(e.contains("`text/` isn't a well-formed content type"), "{e}");
        let e = err(r#"needs /net Listener  write /net "a"  read /net/a/b"#);
        assert!(e.contains("`/net` serves nothing at `/a/b`"), "the runtime's objects have headers too: {e}");
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
        let files = r#"needs /f Files  write /f ".""#;
        let e = err_with(&format!("{HEADER} {files} instance e : Echo {{ bind /dev/cons /f }}"), &modules);
        assert!(e.contains("`e` needs `/ watch -> text/plain*` at `/dev/cons`, and `/f` serves `/ operator write text/plain, read -> */*, stat, list`, `/** read -> */*, stat, list`"), "{e}");
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
        let net = r#"needs /net Listener  write /net "127.0.0.1:0""#;
        let img = format!("{net} instance site : Site instance http : Http {{ bind /net/tcp /net  mount /www site  mount /other site }}");
        assert!(check_with(&img, &modules).is_ok(), "{:#}", check_with(&img, &modules).unwrap_err());

        // /hits answers a read with text/plain, where Strict needs text/html everywhere.
        let e = err_with("instance site : Site instance s : Strict { mount /www site }", &modules);
        assert!(e.contains("`s` needs `/** read -> text/html` at `/www`, and `site` serves `/hits read -> text/plain`"), "{e}");
    }

    /// An image with an instance of A, which needs /out and serves `/x`
    /// and `/y`, exporting `/x` alone.
    const SITE: &str = r#"
        import { A } from "./A.oth"
        require greeting : text/plain = "hi"
        require name : text/plain
        needs /out { / write text/plain }
        instance a : A { bind /out /out }
        mount /a a
        write /a/x greeting
        write /a/y name
        export /a/x at /exported
    "#;
    const A: &str = "object A { needs /out { / write text/plain }  serves { /x write text/plain  /y write text/plain  /z read } }";

    fn with_site(img: &str) -> Result<Program> {
        let b = "object B { needs /web { /exported write text/plain } }";
        check_all(&format!(r#"import Site from "./site.oth" {img}"#), &[("A", A), ("B", b)], &[("site.oth", SITE)])
    }

    #[test]
    fn an_image_is_an_object_made_of_objects() {
        let p = with_site(r#"instance cons : MemoryConsole  instance web : Site { mount /out cons  name = "x" }  instance b : B { mount /web web }"#)
            .unwrap();
        assert_eq!(names(&p), ["cons", "web", "b", "web.a"], "its instances are named under its own");
        let (web, a) = (1, 3);
        assert_eq!(p.instances[web].mounts, [mount("/out", 0, false), mount("/a", a, false)], "its view: its need, then its own mounts");
        let Kind::Image { exports } = &p.instances[web].kind else { panic!("{:?}", p.instances[web].kind) };
        assert_eq!(*exports, [MountDecl { at: "/exported".into(), target: a, path: "/x".into(), read_only: false }]);
        assert_eq!(p.instances[a].mounts, [mount("/out", 0, false)], "a bind through the need, to what the parent mounted there");
        let served: Vec<String> = p.interfaces[&p.instances[web].type_name].serves.iter().map(|e| e.to_string()).collect();
        assert_eq!(served, ["/exported write text/plain"], "it serves what it exports, and nothing else");

        // Its requests come first, resolved in its own view, with its slots.
        let s: Vec<(Option<usize>, &str, String)> =
            p.startup.iter().map(|s| (s.image, s.path.as_str(), s.body.as_ref().unwrap().text().unwrap())).collect();
        assert_eq!(s, [(Some(web), "/a/x", "hi".into()), (Some(web), "/a/y", "x".into())]);
        let d = describe(&p, &|_| None, &|_| false);
        assert!(d.contains("b      sees /web → web\n       can reach at most: cons rw (via web), web.a rw (via web)\n"), "{d}");
    }

    #[test]
    fn an_image_is_held_to_what_it_needs_and_exports() {
        let e = format!("{:#}", with_site(r#"instance web : Site { name = "x" }"#).unwrap_err());
        assert!(e.contains("`web` needs something mounted at `/out`"), "{e}");
        let c = "object C { needs /web { /y write text/plain } }";
        let e = format!(
            "{:#}",
            check_all(
                r#"import Site from "./site.oth" instance cons : MemoryConsole  instance web : Site { mount /out cons  name = "x" }  instance c : C { mount /web web }"#,
                &[("A", A), ("C", c)],
                &[("site.oth", SITE)]
            )
            .unwrap_err()
        );
        assert!(e.contains("`c` needs `/y write text/plain` at `/web`, and `web` serves nothing there"), "only what it exports: {e}");
        let e = format!("{:#}", with_site(r#"instance cons : MemoryConsole  instance web : Site { mount /out cons }"#).unwrap_err());
        assert!(e.contains("`web` needs a value for its slot `name`: `name = ...` in its block"), "{e}");
        let e = format!("{:#}", with_site(r#"instance cons : MemoryConsole  instance web : Site { mount /out cons  name = "x"  nope = "y" }"#).unwrap_err());
        assert!(e.contains("`Site` has no slot `nope`"), "{e}");
        let e = format!("{:#}", with_site(r#"require b : application/bson  instance cons : MemoryConsole  instance web : Site { mount /out cons  name = b }"#).unwrap_err());
        assert!(e.contains("`web`'s slot `name` is a text/plain, and isn't given one"), "{e}");
        let e = format!("{:#}", with_site(r#"instance cons : MemoryConsole  instance web : Site { mount /out cons  name = text/html "x" }"#).unwrap_err());
        assert!(e.contains("`web`'s slot `name` is a text/plain, and isn't given one"), "{e}");
        let e = format!("{:#}", with_site(r#"instance a : A { name = "x" }"#).unwrap_err());
        assert!(e.contains("`a` is a A, which has no slots; only an image's instance takes values"), "{e}");
    }

    #[test]
    fn an_image_takes_its_slots_from_the_root() {
        let p = with_site(r#"require who : text/plain  instance cons : MemoryConsole  instance web : Site { mount /out cons  name = who }"#).unwrap();
        assert_eq!(p.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["who"], "only the root's are for whoever runs it");
        assert_eq!(p.startup[1].body, Some(Arg::Slot(0)));
    }

    #[test]
    fn binds_and_exports_resolve_through_other_images() {
        let p = with_site(
            r#"instance cons : MemoryConsole  instance web : Site { mount /out cons  name = "x" }  mount /web web
               instance c : C { bind /e /web/exported }"#,
        )
        .unwrap();
        let c = p.instances.iter().position(|i| i.name == "c").unwrap();
        let a = p.instances.iter().position(|i| i.name == "web.a").unwrap();
        assert_eq!(p.instances[c].mounts, [MountDecl { at: "/e".into(), target: a, path: "/x".into(), read_only: false }]);
    }

    #[test]
    fn images_are_imported_by_a_name() {
        let e = err(r#"import "./site.oth""#);
        assert!(e.contains("an image is no longer imported whole: import it by a name, `import Site from \"...\"`"), "{e}");
        let loops = "import Me from \"./loop.oth\" instance me : Me";
        let e = format!("{:#}", check_all(loops, &[], &[("loop.oth", loops)]).unwrap_err());
        assert!(e.contains("images instantiate each other in a loop"), "{e}");
    }

    #[test]
    fn describes_views_and_reach() {
        let p = check_src(
            r#"needs /cons Console
            instance files : PackedFiles
            instance site : Site { mount /files files readonly }
            instance http : Http { mount /www site  bind /dev/cons /cons }"#,
        )
        .unwrap();
        assert_eq!(
            describe(&p, &|_| None, &|i| p.instances[i].name.starts_with('/')),
            "files  sees nothing\n\
             site   sees /files → files ro\n       can reach at most: files ro\n\
             http   sees /www → site, /dev/cons → /cons\n       can reach at most: /cons rw, files ro (via site), site rw\n"
        );
    }
}
