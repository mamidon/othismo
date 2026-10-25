//! Headers: the declarations in `.oth` files, which declare protocols,
//! traits and objects: what each object serves, what it needs and its
//! documentation. See `docs/objects.md`. What a payload holds is its
//! format's business, and its schemas are in files of their own (see the
//! `format` crate).
//!
//! A file's declarations are parsed into a [`File`], then resolved into a
//! [`Header`]: its imports followed, every name it mentions declared, and
//! its guides read in. A resolved header stands on its own, so it can
//! travel inside a module.

use syntax::image_definition::{self, Ident, ImageDefinition, Pos, Source};
pub use image::header::*;
pub use syntax::decl::*;
use format::{Format, Formats, Schemas};
use protocol::pattern::PathPattern;
use protocol::payload::{self, STAT, STAT_NAME};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The runtime's own header, one file at a time: the bodies its objects
/// share, then each trait it implements with its stand-in in memory,
/// PackedFiles, and TestSuite. A file may import from the files before it.
const ENV_SOURCES: &[(&str, &str)] = &[
    ("bodies.oth", include_str!("../../../sdk/env/bodies.oth")),
    ("Console.oth", include_str!("../../../sdk/env/Console.oth")),
    ("Listener.oth", include_str!("../../../sdk/env/Listener.oth")),
    ("Files.oth", include_str!("../../../sdk/env/Files.oth")),
    ("PackedFiles.oth", include_str!("../../../sdk/env/PackedFiles.oth")),
    ("TestSuite.oth", include_str!("../../../sdk/env/TestSuite.oth")),
];

/// The runtime's header, `env`: every file in `sdk/env/`, joined.
pub fn env() -> &'static Header {
    static ENV: OnceLock<Header> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = Header::default();
        let mut done: BTreeMap<&str, Header> = BTreeMap::new();
        for &(name, src) in ENV_SOURCES {
            let file = parse(src).unwrap_or_else(|e| panic!("env/{name} parses: {e:#}"));
            let mut earlier = |s: &Source| -> Result<Header> {
                let found = match s {
                    Source::Path(p) => done.get(p.strip_prefix("./").unwrap_or(p)),
                    Source::Env => None,
                };
                found.cloned().ok_or_else(|| anyhow::anyhow!("env/{name} can only import from the env files before it"))
            };
            let header = resolve(&file, &mut earlier, &|_| bail!("env has no guides"))
                .unwrap_or_else(|e| panic!("env/{name} resolves: {e:#}"));
            env.protocols.extend(header.protocols.clone());
            env.traits.extend(header.traits.clone());
            env.objects.extend(header.objects.iter().cloned());
            done.insert(name, header);
        }
        // Everything in it can be imported.
        env.exports = env.protocols.keys().chain(env.traits.keys()).cloned().collect();
        env.exports.extend(env.objects.iter().map(|o| o.name.clone()));
        env
    })
}

// ---------------------------------------------------------------------------
// In modules

/// The custom section a module's header travels in, as BSON.
pub const SECTION: &str = "othismo.oth";

/// `wasm` with `header` in its [`SECTION`], replacing any already there.
pub fn embed(wasm: &[u8], header: &Header) -> Result<Vec<u8>> {
    embed_section(wasm, SECTION, header).context("encoding the header")
}

/// `wasm` with `schemas`, `format`'s, in its section, replacing any
/// already there. A module without schemas has no section for them.
pub fn embed_schemas(wasm: &[u8], format: &dyn Format, schemas: &Schemas) -> Result<Vec<u8>> {
    embed_bytes(wasm, format.section(), (!schemas.0.is_empty()).then_some(&schemas.0[..]))
}

/// The schemas of `format` inside `wasm`; none if it has no section for them.
pub fn embedded_schemas(wasm: &[u8], format: &dyn Format) -> Result<Schemas> {
    let name = format.section();
    let Some(section) = sections(wasm)?.into_iter().find(|s| s.custom == Some(name)) else { return Ok(Schemas::default()) };
    format.load(section.payload).with_context(|| format!("its `{name}` section is malformed; rebuild it with this othismo's `embed`"))
}

/// `wasm` with `value`, as BSON, in the custom section `name`.
fn embed_section(wasm: &[u8], name: &str, value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bson::serialize_to_document(value)?.to_writer(&mut bytes)?;
    embed_bytes(wasm, name, Some(&bytes))
}

/// `wasm` with `bytes` as the custom section `name`, or with no such
/// section if `bytes` is `None`.
fn embed_bytes(wasm: &[u8], name: &str, bytes: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut out = wasm[..8].to_vec();
    for section in sections(wasm)? {
        if section.custom != Some(name) {
            out.extend_from_slice(section.bytes);
        }
    }
    let Some(bytes) = bytes else { return Ok(out) };
    let mut payload = Vec::new();
    leb128(&mut payload, name.len() as u64);
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(bytes);
    out.push(0);
    leb128(&mut out, payload.len() as u64);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// What the custom section `name` of `wasm` holds, if it has one.
fn section<T: serde::de::DeserializeOwned>(wasm: &[u8], name: &str) -> Result<Option<T>> {
    let Some(section) = sections(wasm)?.into_iter().find(|s| s.custom == Some(name)) else { return Ok(None) };
    let malformed = || format!("its `{name}` section is malformed; rebuild it with this othismo's `embed`");
    let doc = bson::Document::from_reader(section.payload).with_context(malformed)?;
    Ok(Some(bson::deserialize_from_document(doc).with_context(malformed)?))
}

/// Checks that `header` is fit for module `name`: it declares exactly one
/// object, named for the module, and exports it.
pub fn for_module(header: &Header, name: &str) -> Result<()> {
    match header.objects.as_slice() {
        [o] if o.name == name && !header.exports.contains(&o.name) => bail!("it doesn't export `{name}`; write `export object {name}`"),
        [o] if o.name == name => Ok(()),
        [o] => bail!("it declares object `{}`, but the module is `{name}`", o.name),
        [] => bail!("it declares no object; a module's header declares `export object {name}`"),
        _ => bail!("it declares {} objects; a module's header declares just `export object {name}`", header.objects.len()),
    }
}

/// The header in `wasm`'s [`SECTION`], if it has one.
pub fn embedded(wasm: &[u8]) -> Result<Option<Header>> {
    section(wasm, SECTION)
}

struct Section<'a> {
    /// The whole section, id and size included.
    bytes: &'a [u8],
    /// A custom section's name.
    custom: Option<&'a str>,
    /// A custom section's contents after its name.
    payload: &'a [u8],
}

/// Splits a module into its sections, without looking inside any but
/// custom ones.
fn sections(wasm: &[u8]) -> Result<Vec<Section<'_>>> {
    if wasm.len() < 8 || &wasm[..4] != b"\0asm" {
        bail!("not a WebAssembly module");
    }
    let mut out = vec![];
    let mut i = 8;
    while i < wasm.len() {
        let start = i;
        let id = wasm[i];
        i += 1;
        let size = read_leb128(wasm, &mut i)?;
        let body = wasm.get(i..i + size).context("a section runs past the end of the module")?;
        i += size;
        let (mut custom, mut payload) = (None, &body[..0]);
        if id == 0 {
            let mut j = 0;
            let len = read_leb128(body, &mut j)?;
            let name = body.get(j..j + len).context("a custom section's name runs past its end")?;
            custom = std::str::from_utf8(name).ok();
            payload = &body[j + len..];
        }
        out.push(Section { bytes: &wasm[start..i], custom, payload });
    }
    Ok(out)
}

fn leb128(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_leb128(bytes: &[u8], i: &mut usize) -> Result<usize> {
    let mut v: u64 = 0;
    for shift in (0..35).step_by(7) {
        let b = *bytes.get(*i).context("truncated module")?;
        *i += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok(v as usize);
        }
    }
    bail!("malformed module: a size is too long")
}

// ---------------------------------------------------------------------------
// Resolving

fn err_at(id: &Ident, msg: impl fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{}: {msg}", id.pos)
}

/// Resolves `file`. `import` provides the header behind each import's
/// source, and `guide` reads a guide's file, named relative to the header.
pub fn resolve(
    file: &File,
    import: &mut dyn FnMut(&Source) -> Result<Header>,
    guide: &dyn Fn(&str) -> Result<String>,
) -> Result<Header> {
    resolve_with(file, &Types::default(), import, guide)
}

/// The message types a file's schemas blocks declare, as their formats
/// list them, and the content types whose formats have schemas.
#[derive(Debug, Clone, Default)]
pub struct Types {
    pub declared: Vec<DeclaredType>,
    /// A payload of one of these is named by its message type, never by
    /// the content type alone.
    pub structured: Vec<String>,
}

/// One message type a schemas block declares.
#[derive(Debug, Clone)]
pub struct DeclaredType {
    pub name: String,
    pub content_type: String,
    /// Its block is exported.
    pub exported: bool,
    /// Where its block is.
    pub pos: Pos,
}

/// [`resolve`], with the message types `types` gives.
pub fn resolve_with(
    file: &File,
    types: &Types,
    import: &mut dyn FnMut(&Source) -> Result<Header>,
    guide: &dyn Fn(&str) -> Result<String>,
) -> Result<Header> {
    let mut protocols: BTreeMap<String, Protocol> = BTreeMap::new();
    let mut messages: BTreeMap<String, String> = BTreeMap::new();
    let mut traits: BTreeMap<String, Trait> = BTreeMap::new();
    let mut imported: BTreeMap<String, Object> = BTreeMap::new();
    let mut from: BTreeMap<String, String> = BTreeMap::new();
    for imp in &file.imports {
        if let Some(d) = &imp.default {
            reserve(d)?;
            if let Some(prev) = from.insert(d.name.clone(), imp.from.to_string()) {
                bail!(err_at(d, format!("`{}` is already imported from {prev}", d.name)));
            }
        }
        if imp.names.is_empty() {
            continue;
        }
        let header = import(&imp.from).with_context(|| format!("{}: importing from {}", imp.pos, imp.from))?;
        for n in &imp.names {
            let (name, local) = (&n.name, &n.local);
            reserve(local)?;
            if let Some(prev) = from.insert(local.name.clone(), imp.from.to_string()) {
                bail!(err_at(local, format!("`{}` is already imported from {prev}", local.name)));
            }
            let declared = header.protocols.contains_key(&name.name)
                || header.traits.contains_key(&name.name)
                || header.object_named(&name.name).is_some()
                || header.types.contains_key(&name.name);
            if !declared {
                bail!(err_at(name, format!("{} has no protocol, trait, object or message type `{}`", imp.from, name.name)));
            }
            if !header.exports.contains(&name.name) {
                bail!(err_at(name, format!("{} doesn't export `{}`", imp.from, name.name)));
            }
            if let Some(ct) = header.types.get(&name.name) {
                if local.name != name.name {
                    bail!(err_at(local, format!("`{}` is a message type, named on the wire, so it can't be renamed", name.name)));
                }
                messages.insert(name.name.clone(), ct.clone());
            } else if let Some(p) = header.protocols.get(&name.name) {
                protocols.insert(local.name.clone(), p.clone());
            } else if let Some(t) = header.traits.get(&name.name) {
                traits.insert(local.name.clone(), t.clone());
            } else if let Some(o) = header.object_named(&name.name) {
                imported.insert(local.name.clone(), o.clone());
            }
        }
    }

    // A name means one thing, whether a protocol, a trait, an object or a
    // message type.
    let type_ids: Vec<Ident> = types.declared.iter().map(|t| Ident { name: t.name.clone(), pos: t.pos }).collect();
    let mut names: Vec<(&Ident, &str)> = file.protocols.iter().map(|p| (&p.name, "protocol")).collect();
    names.extend(file.traits.iter().map(|t| (&t.name, "trait")));
    names.extend(file.objects.iter().map(|o| (&o.name, "object")));
    names.extend(type_ids.iter().map(|t| (t, "message type")));
    for t in &types.declared {
        messages.insert(t.name.clone(), t.content_type.clone());
    }
    for (i, &(name, kind)) in names.iter().enumerate() {
        reserve(name)?;
        if let Some(prev) = from.get(&name.name) {
            bail!(err_at(name, format!("`{}` is imported from {prev}, and declared here too", name.name)));
        }
        if let Some((_, prev)) = names[..i].iter().find(|(n, _)| n.name == name.name) {
            let what = if *prev == kind { format!("{kind} `{}` is declared twice", name.name) } else {
                let a = |w: &str| format!("{} {w}", if w.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" });
                format!("`{}` is declared as {} and as {}", name.name, a(prev), a(kind))
            };
            bail!(err_at(name, what));
        }
    }

    let mut shapes = Shapes {
        protocol_decls: file.protocols.iter().map(|p| (p.name.name.as_str(), p)).collect(),
        trait_decls: file.traits.iter().map(|t| (t.name.name.as_str(), t)).collect(),
        protocols,
        traits,
        messages,
        structured: &types.structured,
        resolving: vec![],
    };
    for p in &file.protocols {
        shapes.named(&p.name)?;
    }
    for t in &file.traits {
        shapes.named(&t.name)?;
    }

    let mut objects: Vec<Object> = vec![];
    for o in &file.objects {
        let guide = match &o.guide {
            Some((path, pos)) => Some(guide(path).with_context(|| format!("{pos}: reading the guide {path:?}"))?),
            None => None,
        };
        let needs = shapes.needs(&o.needs)?;
        let mut serves = shapes.entries(&o.serves)?;
        for t in &o.traits {
            let Shape::Trait(entries) = shapes.named(t)? else {
                bail!(err_at(t, format!("`{}` is a protocol; an object declares traits, the shapes of trees", t.name)));
            };
            for e in entries {
                merge(&mut serves, e, t.pos)?;
            }
        }
        let traits = o.traits.iter().map(|t| t.name.clone()).collect();
        objects.push(Object { name: o.name.name.clone(), doc: o.doc.clone(), traits, guide, needs, serves, origin: None });
    }
    let image_needs = shapes.needs(&file.needs)?;
    let mut exports: Vec<String> = types.declared.iter().filter(|t| t.exported).map(|t| t.name.clone()).collect();
    for e in &file.exports {
        let known = names.iter().any(|(n, _)| n.name == e.name) || from.contains_key(&e.name);
        if !known {
            bail!(err_at(e, format!("`{}` is neither declared nor imported here, so it can't be exported", e.name)));
        }
        if exports.contains(&e.name) {
            bail!(err_at(e, format!("`{}` is exported twice", e.name)));
        }
        exports.push(e.name.clone());
    }
    // What an exported object takes and answers, whoever imports it can name.
    for o in objects.iter().filter(|o| exports.contains(&o.name)) {
        let entries = o.serves.iter().chain(o.needs.iter().flat_map(|n| &n.entries));
        let named = entries.flat_map(|e| &e.requests).flat_map(|r| r.message.iter().chain(r.answer.iter().flat_map(|a| &a.message)));
        for m in named {
            if let Some(t) = types.declared.iter().find(|t| &t.name == m && !t.exported) {
                bail!(
                    "{}: `{}` is exported, and names `{m}`, but its schemas block isn't: write `export schemas {}`",
                    t.pos,
                    o.name,
                    t.content_type
                );
            }
        }
    }
    let Shapes { protocols, traits, messages, .. } = shapes;
    Ok(Header { protocols, traits, objects, imported, exports, needs: image_needs, types: messages })
}

/// What a protocol's or a trait's name stands for, written out.
enum Shape {
    Protocol(Vec<Request>),
    Trait(Vec<Entry>),
}

/// Resolves a header's protocols and traits, each the first time it is
/// named, so they can be declared in any order.
struct Shapes<'a> {
    protocol_decls: BTreeMap<&'a str, &'a ProtocolDecl>,
    trait_decls: BTreeMap<&'a str, &'a TraitDecl>,
    protocols: BTreeMap<String, Protocol>,
    traits: BTreeMap<String, Trait>,
    /// Every message type it can name, with its content type.
    messages: BTreeMap<String, String>,
    structured: &'a [String],
    /// The ones being resolved, to catch one that contains itself.
    resolving: Vec<String>,
}

impl Shapes<'_> {
    fn named(&mut self, id: &Ident) -> Result<Shape> {
        let name = id.name.as_str();
        if let Some(p) = self.protocols.get(name) {
            return Ok(Shape::Protocol(p.requests.clone()));
        }
        if let Some(t) = self.traits.get(name) {
            return Ok(Shape::Trait(t.entries.clone()));
        }
        if self.resolving.iter().any(|r| r == name) {
            bail!(err_at(id, format!("`{name}` contains itself")));
        }
        self.resolving.push(name.to_string());
        let shape = if let Some(&decl) = self.protocol_decls.get(name) {
            let requests = self.requests(&decl.items, &format!("protocol `{name}`"))?;
            self.protocols.insert(name.into(), Protocol { name: name.into(), doc: decl.doc.clone(), requests: requests.clone() });
            Shape::Protocol(requests)
        } else if let Some(&decl) = self.trait_decls.get(name) {
            let entries = self.entries(&decl.entries)?;
            self.traits.insert(name.into(), Trait { name: name.into(), doc: decl.doc.clone(), entries: entries.clone() });
            Shape::Trait(entries)
        } else {
            bail!(err_at(id, format!("unknown protocol or trait `{name}`; is it imported?")));
        };
        self.resolving.pop();
        Ok(shape)
    }

    /// The requests `items` stand for, where `at` names the place for errors.
    /// A request written twice is a mistake; one that two protocols both
    /// list is fine, so long as they agree on its answer.
    fn requests(&mut self, items: &[Item], at: &str) -> Result<Vec<Request>> {
        let mut out: Vec<(Request, bool)> = vec![];
        for item in items {
            let (reqs, written, pos) = match item {
                Item::Request(r) => (vec![self.request(r)?], true, r.pos),
                Item::Named(id) => match self.named(id)? {
                    Shape::Protocol(reqs) => (reqs, false, id.pos),
                    Shape::Trait(_) => bail!(err_at(
                        id,
                        format!("`{}` is a trait; it stands alone at a path, and puts its whole tree there", id.name)
                    )),
                },
            };
            for req in reqs {
                match out.iter().find(|(prev, _)| prev.verb == req.verb && prev.body == req.body && prev.message == req.message && prev.operator == req.operator) {
                    Some((prev, prev_written)) if *prev_written && written => bail!("{pos}: {at} takes `{req}` twice"),
                    Some((prev, _)) if *prev == req => {}
                    Some((prev, _)) => bail!("{pos}: {at} takes `{prev}` and `{req}`, which disagree"),
                    None => out.push((req, written)),
                }
            }
        }
        Ok(out.into_iter().map(|(r, _)| r).collect())
    }

    fn request(&self, r: &RequestDecl) -> Result<Request> {
        if let Some(implied) = Request::implied(r.verb) {
            if r.body.is_some() || r.answer.is_some() {
                bail!("{}: `{}` takes nothing and answers {}; write it bare", r.pos, r.verb, if implied.stream { "a stream of stat records" } else { "a stat record" });
            }
            return Ok(Request { operator: r.operator, verb: r.verb, body: None, message: None, answer: Some(implied) });
        }
        let (body, message) = match &r.body {
            Some(id) => {
                let (ty, message) = self.payload_type(id)?;
                (Some(ty), message)
            }
            None => (None, None),
        };
        let answer = match &r.answer {
            Some((id, stream)) => {
                let (ty, message) = self.payload_type(id)?;
                Some(Answer { ty, message, stream: *stream })
            }
            None => None,
        };
        Ok(Request { operator: r.operator, verb: r.verb, body, message, answer })
    }

    /// The content type `id` names, and the message type if it names one:
    /// a message type, `Stat`, or a content type that has no schemas.
    fn payload_type(&self, id: &Ident) -> Result<(String, Option<String>)> {
        if let Some(ty) = self.messages.get(&id.name) {
            return Ok((ty.clone(), Some(id.name.clone())));
        }
        let ty = content_type(id)?;
        if self.structured.contains(&ty) {
            bail!(err_at(id, format!("`{ty}` payloads are named by their message type, declared in a `schemas {ty}` block, such as `write Greeting`")));
        }
        Ok((ty, None))
    }

    /// The needs blocks `decls` stand for. An object can't make the
    /// operator's requests, so a need never includes them, even where it
    /// names a trait that serves some.
    fn needs(&mut self, decls: &[NeedDecl]) -> Result<Vec<Need>> {
        let mut needs: Vec<Need> = vec![];
        for n in decls {
            let at = pattern(&n.at)?;
            if let Some(prev) = needs.iter().find(|p| same(&p.at, &at)) {
                bail!("{}: `needs {}` overlaps `needs {}` exactly; one block per mount point", n.at.pos, at, prev.at);
            }
            let mut entries = self.entries(&n.entries)?;
            for e in &mut entries {
                e.requests.retain(|r| !r.operator);
            }
            entries.retain(|e| !e.requests.is_empty());
            let shape = match n.entries.as_slice() {
                [e] if e.path.path == "/" => match e.items.as_slice() {
                    [Item::Named(id)] => self.traits.get(&id.name).map(|t| t.name.clone()),
                    _ => None,
                },
                _ => None,
            };
            needs.push(Need { at, doc: n.doc.clone(), entries, shape });
        }
        Ok(needs)
    }

    /// The entries `decls` stand for, with each trait's tree put at its path.
    fn entries(&mut self, decls: &[EntryDecl]) -> Result<Vec<Entry>> {
        let mut written: Vec<PathPattern> = vec![];
        let mut out: Vec<Entry> = vec![];
        for e in decls {
            let path = pattern(&e.path)?;
            if let Some(prev) = written.iter().find(|p| same(p, &path)) {
                bail!("{}: `{path}` is already listed, as `{prev}`", e.path.pos);
            }
            written.push(path.clone());
            if let [Item::Named(id)] = e.items.as_slice()
                && let Shape::Trait(entries) = self.named(id)?
            {
                for t in entries {
                    let doc = if t.path.as_str() == "/" && t.doc.is_empty() { e.doc.clone() } else { t.doc };
                    merge(&mut out, Entry { path: rebase(&path, &t.path, id.pos)?, doc, requests: t.requests }, id.pos)?;
                }
                continue;
            }
            let requests = self.requests(&e.items, &format!("`{path}`"))?;
            merge(&mut out, Entry { path, doc: e.doc.clone(), requests }, e.path.pos)?;
        }
        Ok(out)
    }
}

/// Adds `entry` to `entries`. Where they already have its pattern, the two
/// must take exactly the same requests.
fn merge(entries: &mut Vec<Entry>, entry: Entry, pos: Pos) -> Result<()> {
    let Some(prev) = entries.iter_mut().find(|p| same(&p.path, &entry.path)) else {
        entries.push(entry);
        return Ok(());
    };
    let set = |e: &Entry| {
        let mut reqs: Vec<String> = e.requests.iter().map(|r| r.to_string()).collect();
        reqs.sort();
        reqs
    };
    if set(prev) != set(&entry) {
        bail!("{pos}: `{}` is given twice, as `{prev}` and as `{entry}`; where traits meet they must agree exactly", entry.path);
    }
    if prev.doc.is_empty() {
        prev.doc = entry.doc;
    }
    Ok(())
}

/// `inner`, a pattern in a trait's tree, as a pattern under `at`, where the
/// trait is put.
fn rebase(at: &PathPattern, inner: &PathPattern, pos: Pos) -> Result<PathPattern> {
    if at.as_str().ends_with("**") {
        bail!("{pos}: a trait can't be put under `{at}`; `**` only comes last");
    }
    let text = match (at.as_str(), inner.as_str()) {
        (_, "/") => at.as_str().to_string(),
        ("/", inner) => inner.to_string(),
        (at, inner) => format!("{}{inner}", at.trim_end_matches('/')),
    };
    PathPattern::parse(&text).map_err(|e| anyhow::anyhow!("{pos}: {e}"))
}

fn reserve(name: &Ident) -> Result<()> {
    if name.name == STAT_NAME {
        bail!(err_at(name, format!("`{STAT_NAME}` is reserved: it stands for a stat record's content type, {STAT}")));
    }
    Ok(())
}

/// The content type `id` writes: as it is, or `Stat` for a stat record's.
fn content_type(id: &Ident) -> Result<String> {
    if id.name == STAT_NAME {
        return Ok(STAT.into());
    }
    if !payload::valid(&id.name) {
        let hint = if id.name.contains('/') {
            "it isn't a well-formed content type"
        } else {
            "it is neither a content type, such as `text/plain`, nor a message type declared in or imported into this file"
        };
        bail!(err_at(id, format!("`{}`: {hint}", id.name)));
    }
    Ok(id.name.clone())
}

fn pattern(p: &image_definition::Path) -> Result<PathPattern> {
    PathPattern::parse(&p.path).map_err(|e| anyhow::anyhow!("{}: {e}", p.pos))
}

/// Two patterns that match exactly the same paths.
fn same(a: &PathPattern, b: &PathPattern) -> bool {
    a.specificity(b) == Ordering::Equal && a.covers(b) && b.covers(a)
}

/// `dir` followed by the relative path `rel`, without its `.` segments.
pub fn join(dir: &Path, rel: &str) -> PathBuf {
    let mut out = dir.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Reads and resolves the header at `path`: a `.oth` file's declarations,
/// or the header inside a module.
pub fn load(path: &Path) -> Result<Header> {
    Loader::default().header(path)
}

/// Reads `.oth` files and the headers inside modules, each once, following
/// imports relative to the file that makes them, and their schemas blocks
/// in the formats it knows.
#[derive(Default)]
pub struct Loader {
    /// The formats whose schemas it reads.
    pub formats: Formats,
    /// What to read instead of the file on disk, by path: for tests.
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    /// Every `.oth` file read, in the order they were first read.
    pub sources: Vec<PathBuf>,
    headers: BTreeMap<PathBuf, Header>,
    schemas: BTreeMap<(PathBuf, &'static str), Schemas>,
    /// The `.oth` files being resolved, to catch one that imports itself.
    loading: Vec<PathBuf>,
    /// The `.oth` files whose schemas are being read, likewise.
    loading_schemas: Vec<PathBuf>,
}

impl Loader {
    pub fn new(formats: Formats) -> Loader {
        Loader { formats, ..Loader::default() }
    }

    pub fn read(&self, path: &Path) -> Result<Vec<u8>> {
        match self.files.get(path) {
            Some(bytes) => Ok(bytes.clone()),
            None => std::fs::read(path).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The `.oth` file at `path`, parsed whole.
    pub fn image_definition(&mut self, path: &Path) -> Result<ImageDefinition> {
        let src = String::from_utf8(self.read(path)?).with_context(|| format!("{} isn't UTF-8", path.display()))?;
        if !self.sources.iter().any(|p| p == path) {
            self.sources.push(path.to_path_buf());
        }
        image_definition::parse(&src).with_context(|| format!("parsing {}", path.display()))
    }

    /// The header at `path`: a `.oth` file's declarations, or the header
    /// inside a module (`.wasm`).
    pub fn header(&mut self, path: &Path) -> Result<Header> {
        if let Some(h) = self.headers.get(path) {
            return Ok(h.clone());
        }
        let header = match path.extension().and_then(|e| e.to_str()) {
            Some("wasm") => {
                let Ok(wasm) = self.read(path) else {
                    let name = path.file_stem().unwrap_or_default().to_string_lossy();
                    bail!("cannot read {} (did you compile {name}.ts?)", path.display());
                };
                let mut header = embedded(&wasm)
                    .with_context(|| path.display().to_string())?
                    .ok_or_else(|| anyhow::anyhow!("{} has no header; embed one with `othismo embed`", path.display()))?;
                for o in &mut header.objects {
                    o.origin = Some(path.to_path_buf());
                }
                header
            }
            Some("oth") => {
                if self.loading.iter().any(|p| p == path) {
                    bail!("{} imports itself, through the files it imports", path.display());
                }
                self.loading.push(path.to_path_buf());
                let header = self.image_definition(path).and_then(|img| self.resolve(&img.header, path));
                self.loading.pop();
                header?
            }
            _ => bail!("{} is neither a `.oth` file nor a module (`.wasm`)", path.display()),
        };
        self.headers.insert(path.to_path_buf(), header.clone());
        Ok(header)
    }

    /// The message types the schemas blocks of `file`, at `path`, declare.
    fn types(&mut self, file: &File, path: &Path) -> Result<Types> {
        let formats = self.formats.clone();
        let mut types = Types { declared: vec![], structured: formats.iter().map(|f| f.content_type().to_string()).collect() };
        for block in &file.schemas {
            let Some(f) = formats.iter().find(|f| f.content_type() == block.ty.name) else {
                let known: Vec<&str> = formats.iter().map(|f| f.content_type()).collect();
                bail!(
                    "{}: in {}: no format reads `{}` schemas; this othismo's are for {}",
                    block.ty.pos,
                    path.display(),
                    block.ty.name,
                    if known.is_empty() { "nothing".to_string() } else { known.join(", ") }
                );
            };
            let schemas = self.schemas(f, path)?;
            for name in f.declared(&schemas) {
                types.declared.push(DeclaredType { name, content_type: block.ty.name.clone(), exported: block.exported, pos: block.ty.pos });
            }
        }
        Ok(types)
    }

    /// `format`'s schemas at `path`: the `.oth` file's block for its content
    /// type, or those inside a module. A file or module with none has none.
    pub fn schemas(&mut self, format: &dyn Format, path: &Path) -> Result<Schemas> {
        let key = (path.to_path_buf(), format.content_type());
        if let Some(m) = self.schemas.get(&key) {
            return Ok(m.clone());
        }
        let schemas = match path.extension().and_then(|e| e.to_str()) {
            Some("wasm") => embedded_schemas(&self.read(path)?, format).with_context(|| path.display().to_string())?,
            Some("oth") => {
                let block = self.image_definition(path)?.header.schemas.into_iter().find(|b| b.ty.name == format.content_type());
                let Some(block) = block else { return Ok(Schemas::default()) };
                if self.loading_schemas.iter().any(|p| p == path) {
                    bail!("{}'s `{}` schemas import themselves, through the files they import", path.display(), block.ty.name);
                }
                let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
                self.loading_schemas.push(path.to_path_buf());
                let parsed = format.parse(&block.positioned_body(), &mut |p| self.schemas(format, &join(&dir, p)));
                self.loading_schemas.pop();
                parsed.with_context(|| format!("in {}", path.display()))?
            }
            _ => bail!("{} is neither a `.oth` file nor a module (`.wasm`), where schemas are", path.display()),
        };
        self.schemas.insert(key, schemas.clone());
        Ok(schemas)
    }

    /// Resolves `file`, the declarations of the `.oth` file at `path`.
    pub fn resolve(&mut self, file: &File, path: &Path) -> Result<Header> {
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let types = self.types(file, path)?;
        // Guides are read first, since resolving borrows the loader for imports.
        let mut guides: BTreeMap<String, Result<String, String>> = BTreeMap::new();
        for (g, _) in file.objects.iter().filter_map(|o| o.guide.as_ref()) {
            let text = self.read(&dir.join(g)).map_err(|e| format!("{e:#}")).and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()));
            guides.insert(g.clone(), text);
        }
        let guide = |g: &str| -> Result<String> { guides[g].clone().map_err(|e| anyhow::anyhow!(e)) };
        let mut import = |s: &Source| -> Result<Header> {
            match s {
                Source::Env => Ok(env().clone()),
                Source::Path(p) => self.header(&join(&dir, p)),
            }
        };
        let mut header = resolve_with(file, &types, &mut import, &guide).with_context(|| format!("in {}", path.display()))?;
        for o in &mut header.objects {
            o.origin = Some(path.to_path_buf());
        }
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{status::Status, verb::Verb};
    use protocol::payload::ANY;

    const GREETER: &str = r#"
        ## Greets every name typed on the console.
        ##
        ## And reports statistics.
        object Greeter {
          ## The console, for names and greetings.
          needs /dev/cons {
            /  watch -> text/plain*, write */*
          }
          needs /shouter { / write text/plain }

          serves {
            ## Configures the greeting.
            /greeting  write application/bson
            /stats     read -> application/bson
          }
        }
    "#;

    fn resolve_str(src: &str) -> Result<Header> {
        let file = parse(src)?;
        let mut import = |s: &Source| -> Result<Header> {
            match s {
                Source::Env => Ok(env().clone()),
                other => bail!("no {other} here"),
            }
        };
        resolve(&file, &mut import, &|p| Ok(format!("the guide in {p}")))
    }

    fn err(src: &str) -> String {
        format!("{:#}", resolve_str(src).unwrap_err())
    }

    const SHAPES: &str = r#"
        ## A site, declared before the trees it is made of.
        export trait Site {
          /hits     read -> text/plain
          ## The static files.
          /static   Tree
          /**       File
        }

        export trait Tree {
          /     Dir
          /**   File, Dir
        }

        export protocol File { read -> */*, stat }
        protocol Dir {
          list,
          stat,
        }

        object S : Site {
          needs /files Tree
          serves { /hits read -> text/plain }
        }
    "#;

    #[test]
    fn protocols_and_traits_are_written_out() {
        let h = resolve_str(SHAPES).unwrap();
        let show = |entries: &[Entry]| -> Vec<String> { entries.iter().map(|e| e.to_string()).collect() };
        assert_eq!(h.protocols["Dir"].requests.iter().map(|r| r.to_string()).collect::<Vec<_>>(), ["list", "stat"]);
        assert_eq!(show(&h.traits["Tree"].entries), ["/ list, stat", "/** read -> */*, stat, list"]);
        let s = h.object("S").unwrap();
        assert_eq!(s.traits, ["Site"]);
        assert_eq!(
            show(&s.serves),
            ["/hits read -> text/plain", "/static list, stat", "/static/** read -> */*, stat, list", "/** read -> */*, stat"],
            "its own `/hits` agrees with Site's, so it is listed once"
        );
        assert_eq!(s.serves[1].doc, "The static files.", "the trait's root takes the doc of where it is put");
        assert_eq!(show(&s.needs[0].entries), show(&h.traits["Tree"].entries));
    }

    #[test]
    fn protocols_and_traits_can_be_imported() {
        let file = parse(r#"import { Tree as Files, File } from "./shapes.oth" object T { needs /f Files serves { /x File } }"#).unwrap();
        let mut import = |_: &Source| resolve_str(SHAPES);
        let h = resolve(&file, &mut import, &|_| unreachable!()).unwrap();
        assert_eq!(h.object("T").unwrap().needs[0].entries.len(), 2);
        assert_eq!(h.object("T").unwrap().serves[0].to_string(), "/x read -> */*, stat");
        assert_eq!(h.traits["Files"].name, "Tree", "a renamed trait is the same trait");

        let unexported = parse(r#"import { Dir } from "./shapes.oth""#).unwrap();
        let e = format!("{:#}", resolve(&unexported, &mut import, &|_| unreachable!()).unwrap_err());
        assert!(e.contains("\"./shapes.oth\" doesn't export `Dir`"), "{e}");
    }

    #[test]
    fn shapes_must_agree_where_they_meet() {
        let e = err("protocol A { read -> text/plain } protocol B { read -> text/html } object O { serves { /x A, B } }");
        assert!(e.contains("`/x` takes `read -> text/plain` and `read -> text/html`, which disagree"), "{e}");
        let e = err("trait T { /x read -> text/plain } trait U { /x read -> text/html } object O : T, U {}");
        assert!(e.contains("`/x` is given twice, as `/x read -> text/plain` and as `/x read -> text/html`; where traits meet they must agree exactly"), "{e}");
        let e = err("trait T { /x read -> text/plain } object O : T { serves { /x read -> text/plain, stat } }");
        assert!(e.contains("where traits meet they must agree exactly"), "{e}");
        assert!(resolve_str("protocol A { read -> */* } protocol B { read -> */* } object O { serves { /x A, B } }").is_ok());
    }

    #[test]
    fn rejects_misused_shapes() {
        assert!(err("protocol A { B } protocol B { A }").contains("`A` contains itself"));
        assert!(err("trait T { /x read } object O { serves { /x T, read } }").contains("`T` is a trait; it stands alone"));
        assert!(err("trait T { /x read } object O { serves { /a/** T } }").contains("a trait can't be put under `/a/**`"));
        assert!(err("protocol P { read } object O : P {}").contains("`P` is a protocol; an object declares traits"));
        assert!(err("object O : Nope {}").contains("unknown protocol or trait `Nope`"));
        assert!(err("object X {} protocol X { read }").contains("`X` is declared as a protocol and as an object"));
        assert!(err("trait X { / read } trait X { / stat }").contains("trait `X` is declared twice"));
        assert!(err("import { File } from env protocol File { read }").contains("`File` is imported from env, and declared here too"));
        assert!(err("protocol P { read, read }").contains("protocol `P` takes `read` twice"));
    }

    #[test]
    fn parses_and_resolves_a_header() {
        let h = resolve_str(GREETER).unwrap();
        let o = h.object("Greeter").unwrap();
        assert_eq!(o.doc, "Greets every name typed on the console.\n\nAnd reports statistics.");
        assert_eq!(o.needs.len(), 2);
        assert_eq!((o.needs[0].at.as_str(), o.needs[0].doc.as_str()), ("/dev/cons", "The console, for names and greetings."));
        let reqs: Vec<String> = o.needs[0].entries[0].requests.iter().map(|r| r.to_string()).collect();
        assert_eq!(reqs, ["watch -> text/plain*", "write */*"]);
        assert_eq!(o.serves[0].doc, "Configures the greeting.");
        assert_eq!(o.serves[1].requests[0].to_string(), "read -> application/bson");
        assert_eq!(o.guide, None);
    }

    #[test]
    fn needs_leave_out_the_operators_requests() {
        let h = resolve_str("trait T { / operator write text/plain  /x operator remove, read } object A { needs /t T }").unwrap();
        let entries: Vec<String> = h.objects[0].needs[0].entries.iter().map(|e| e.to_string()).collect();
        assert_eq!(entries, ["/x read"]);
    }

    #[test]
    fn guides_are_read_in() {
        let h = resolve_str(r#"object A { guide "./A.md" }"#).unwrap();
        assert_eq!(h.objects[0].guide.as_deref(), Some("the guide in ./A.md"));
    }

    #[test]
    fn env_declares_traits_and_packed_files() {
        let env = env();
        for name in ["Console", "Listener", "Files", "FileTree"] {
            assert!(env.traits.contains_key(name), "{name}");
        }
        let objects: Vec<&str> = env.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(objects, ["MemoryConsole", "MemoryListener", "PackedFiles"], "the rest are needed by trait");
    }

    /// Resolves `files`, each a path and its source, with BSON, and returns
    /// the header of the first.
    fn load_all(files: &[(&str, &str)]) -> Result<Header> {
        let mut loader = Loader::new(Formats::new(vec![Box::new(format_bson::Bson)]));
        for (path, src) in files {
            loader.files.insert(path.into(), src.as_bytes().to_vec());
        }
        loader.header(Path::new(files[0].0))
    }

    #[test]
    fn requests_name_message_types() {
        let h = load_all(&[(
            "/t/A.oth",
            "export object A { serves { /greeting write Greeting, read -> Greeting  /any write */* } }
             export schemas application/bson { message Greeting { text: string } }",
        )])
        .unwrap();
        let r = &h.objects[0].serves[0].requests;
        assert_eq!((r[0].body.as_deref(), r[0].message.as_deref()), (Some("application/bson"), Some("Greeting")));
        assert_eq!(r[1].answer.as_ref().and_then(|a| a.message.as_deref()), Some("Greeting"));
        assert_eq!(r[0].to_string(), "write Greeting");
        assert_eq!(h.types["Greeting"], "application/bson");
        assert!(h.exports.contains(&"Greeting".to_string()), "an exported block exports its types");

        // Imported, by the name it has on the wire.
        let b = "import { Greeting } from \"./A.oth\" export object B { needs /a { /greeting write Greeting } }";
        let a = "export object A {} export schemas application/bson { message Greeting { text: string } }";
        let h = load_all(&[("/t/B.oth", b), ("/t/A.oth", a)]).unwrap();
        assert_eq!(h.objects[0].needs[0].entries[0].requests[0].message.as_deref(), Some("Greeting"));
        let e = format!("{:#}", load_all(&[("/t/B.oth", &b.replace("{ Greeting }", "{ Greeting as G }")), ("/t/A.oth", a)]).unwrap_err());
        assert!(e.contains("`Greeting` is a message type, named on the wire, so it can't be renamed"), "{e}");
        let private = "export object A {} schemas application/bson { message Greeting { text: string } }";
        let e = format!("{:#}", load_all(&[("/t/B.oth", b), ("/t/A.oth", private)]).unwrap_err());
        assert!(e.contains("doesn't export `Greeting`"), "{e}");
    }

    #[test]
    fn a_payload_with_schemas_is_named_by_its_type() {
        let e = |src: &str| format!("{:#}", load_all(&[("/t/A.oth", src)]).unwrap_err());
        let bare = e("export object A { serves { /x write application/bson } }");
        assert!(bare.contains("1:37: `application/bson` payloads are named by their message type"), "{bare}");
        let hidden = e("export object A { serves { /x write M } }\nschemas application/bson { message M {} }");
        assert!(hidden.contains("2:9: `A` is exported, and names `M`, but its schemas block isn't: write `export schemas application/bson`"), "{hidden}");
        let clash = e("export object M {}\nschemas application/bson { message M {} }");
        assert!(clash.contains("`M` is declared as an object and as a message type"), "{clash}");
        let unknown = e("schemas application/json { }");
        assert!(unknown.contains("no format reads `application/json` schemas; this othismo's are for application/bson"), "{unknown}");
        // Payloads without schemas are named by their content type, as before.
        assert!(load_all(&[("/t/A.oth", "object A { serves { /x write text/plain, read -> image/png } }")]).is_ok());
        assert!(load_all(&[("/t/A.oth", "object A { serves { /x write M } }\nschemas application/bson { message M {} }")]).is_ok(), "an object that isn't exported");
    }

    #[test]
    fn rejects_mistakes() {
        assert!(err("message A { x: i32 }").contains("message types are declared in their format's block"));
        assert!(err("protocol Stat { read }").contains("reserved"));
        assert!(err("import { Nope } from env").contains("env has no protocol, trait, object or message type `Nope`"));
        assert!(err(r#"import { X } from "./other.oth""#).contains("no \"./other.oth\" here"));
        assert!(err("import { X } from Other").contains("import from a file, relative to this one, such as `from \"./scripts/Other.oth\"`"));
        assert!(err("protocol P { read } export { P, Q }").contains("`Q` is neither declared nor imported here"));
        assert!(err("export protocol P { read } export { P }").contains("`P` is exported twice"));
        assert!(err("object A {} object A {}").contains("object `A` is declared twice"));
        let e = err("object A { serves { /x write Greeting } }");
        assert!(e.contains("`Greeting`: it is neither a content type, such as `text/plain`, nor a message type declared in or imported into this file"), "{e}");
        assert!(err("object A { serves { /x read -> text/ } }").contains("isn't a well-formed content type"));
        assert!(err("object A { serves { /x read, read } }").contains("takes `read` twice"));
        assert!(err("object A { serves { /{a} read /{b} stat } }").contains("already listed"));
        assert!(err("object A { needs /** { / read } needs /** { / stat } }").contains("one block per mount point"));
        assert!(err("object A { serves { /a/**/b read } }").contains("`**` can only come last"));
        assert!(err("object A { serves { /x frob } }").contains("unknown protocol or trait `frob`"));
        assert!(err("object A { serves { /x } }").contains("expected a verb"));
        assert!(err("object A { serves { /x stat -> Stat } }").contains("`stat` takes nothing and answers a stat record; write it bare"));
        assert!(err("object A { serves { /x list text/plain } }").contains("answers a stream of stat records"));
        assert!(err("## nothing\n").contains("documents what follows"));
        assert!(err("## an import\nimport { File } from env").contains("documents what follows"));
        assert!(err("object A { guide \"a\" guide \"b\" }").contains("names its guide twice"));
    }

    #[test]
    fn requests_take_payloads_by_content_type() {
        let h = resolve_str("object A { serves { /h operator remove text/plain, operator remove, read -> */**, write */*, create text/* -> Stat } }").unwrap();
        let e = &h.objects[0].serves[0];
        assert_eq!(e.takes(Verb::Remove, Some("text/plain")).unwrap().body.as_deref(), Some("text/plain"));
        assert_eq!(e.takes(Verb::Remove, None).unwrap().body, None);
        assert_eq!(e.takes(Verb::Remove, Some("application/bson")), Err(Status::INVALID));
        assert_eq!(e.takes(Verb::Write, Some("image/png")).unwrap().body.as_deref(), Some(ANY));
        assert_eq!(e.takes(Verb::Write, None), Err(Status::INVALID));
        assert_eq!(e.takes(Verb::Create, Some("text/html")).unwrap().answer.as_ref().unwrap().ty, STAT);
        assert_eq!(e.takes(Verb::Stat, None), Err(Status::NOT_SUPPORTED));
        assert_eq!(e.takes_verb(Verb::Read, false), Ok(()));
        assert_eq!(e.takes_verb(Verb::Remove, false), Err(Status::FORBIDDEN), "only the operator may remove");
        assert_eq!(e.takes_verb(Verb::Remove, true), Ok(()));
        assert_eq!(e.takes_verb(Verb::Stat, true), Err(Status::NOT_SUPPORTED));
        assert!(e.streams(Verb::Read) && !e.streams(Verb::Write));
        assert_eq!(e.to_string(), "/h operator remove text/plain, operator remove, read -> */**, write */*, create text/* -> Stat");
        assert!(err("object A { serves { /h control text/plain } }").contains("`control` is gone"));
    }

    #[test]
    fn wildcards_meet_needs_in_one_direction() {
        let r = |src: &str| {
            let h = resolve_str(&format!("object A {{ serves {{ / {src} }} }}")).unwrap();
            h.objects[0].serves[0].requests[0].clone()
        };
        assert!(r("write */*").meets(&r("write text/plain")), "a server taking anything takes text");
        assert!(r("write text/*").meets(&r("write text/plain")));
        assert!(!r("write text/plain").meets(&r("write */*")), "but a need to send anything isn't met by one taking text");
        assert!(r("read -> text/html").meets(&r("read -> */*")));
        assert!(r("read -> text/html").meets(&r("read -> text/*")));
        assert!(r("remove").meets(&r("remove -> */*")), "no answer is fine where any is");
        assert!(!r("read -> */*").meets(&r("read -> text/html")));
        assert!(!r("watch -> text/plain").meets(&r("watch -> text/plain*")), "a stream isn't a single answer");
        assert!(!r("watch -> text/plain*").meets(&r("watch -> */*")));
        assert!(r("watch -> text/plain*").meets(&r("watch -> */**")));
        assert!(!r("write text/plain").meets(&r("write")));
        assert!(!r("read").meets(&r("stat")));
        assert!(r("list").meets(&r("list")));
        assert!(!r("operator write */*").meets(&r("write text/plain")), "an object's need is never met by the operator's request");
    }

    #[test]
    fn picks_the_most_specific_entry_and_need() {
        let h = resolve_str(
            "object A {
              needs /net/tcp { /{conn} write application/octet-stream }
              needs /** { /** read -> */* }
              serves { /hits read -> text/plain  /** read -> */* }
            }",
        )
        .unwrap();
        let o = &h.objects[0];
        let path = |text| protocol::path::PathName::parse(text).unwrap();
        let at = |text| protocol::path::MountPoint::parse(text).unwrap();
        assert_eq!(o.serving(&path("/hits")).unwrap().path.as_str(), "/hits");
        assert_eq!(o.serving(&path("/")).unwrap().path.as_str(), "/**");
        assert_eq!(o.need(&at("/net/tcp")).unwrap().at.as_str(), "/net/tcp");
        assert_eq!(o.need(&at("/www")).unwrap().at.as_str(), "/**");
        assert!(o.need(&at("/net/tcp")).unwrap().entry(&path("/")).is_none(), "`{{conn}}` is never empty");
    }

    #[test]
    fn headers_travel_in_modules() {
        let h = resolve_str(GREETER).unwrap();
        let wasm = wat::parse_str(r#"(module (@custom "other" "x") (func (export "f")))"#).unwrap();
        assert_eq!(embedded(&wasm).unwrap(), None);
        let with = embed(&wasm, &h).unwrap();
        assert_eq!(embedded(&with).unwrap(), Some(h.clone()));
        let mut other = h.clone();
        other.objects.clear();
        let again = embed(&with, &other).unwrap();
        assert_eq!(embedded(&again).unwrap(), Some(other), "embedding again replaces the header");
        assert_eq!(sections(&again).unwrap().len(), sections(&with).unwrap().len());
        assert!(sections(&again).unwrap().iter().any(|s| s.custom == Some("other")), "other sections are kept");
        wasmtime::Module::new(&wasmtime::Engine::default(), &again).unwrap();
        assert!(embedded(b"(module)").is_err());
    }

    #[test]
    fn headers_round_trip_through_bson() {
        let h = resolve_str(GREETER).unwrap();
        let doc = bson::serialize_to_document(&h).unwrap();
        let back: Header = bson::deserialize_from_document(doc).unwrap();
        assert_eq!(back, h);
    }
}
