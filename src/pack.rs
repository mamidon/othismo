//! A built image: a checked [`Program`] plus everything it needs from disk,
//! held in memory. The runtime only ever runs a `Pack`.
//!
//! A pack comes from building a `.oth` ([`build`]) or from opening a sealed
//! `.rti` file ([`open`]). The file is a magic number, a BSON manifest and
//! the blobs it refers to (see `docs/images.md`):
//!
//! ```text
//! "RTIMAGE" version   u64 LE manifest length   manifest   blobs
//! ```
//!
//! Blobs are named by the SHA-256 of their contents and stored once.

use crate::check::{self, Kind, Program};
use crate::render;
use crate::tree::{Node, Tree};
use anyhow::{Context, Result, bail};
use format::Formats;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

/// The magic number: `RTIMAGE`, then the format version.
pub const MAGIC: &[u8; 8] = b"RTIMAGE\x07";

/// The format version, bumped whenever the manifest's layout changes. An
/// image built in another format is refused, with a request to rebuild it.
pub const VERSION: u8 = MAGIC[7];

/// The hex SHA-256 of a blob's contents.
pub type Hash = String;

#[derive(Debug)]
pub struct Pack {
    pub program: Program,
    /// Compiled modules by instance type.
    pub modules: BTreeMap<String, Hash>,
    /// Reference copies of the image's sources, by path relative to the
    /// image. The runtime never reads them.
    pub sources: BTreeMap<String, Hash>,
    /// Each `Packed.Files` instance's files, by instance name.
    pub trees: BTreeMap<String, Tree>,
    pub blobs: BTreeMap<Hash, Rc<[u8]>>,
    /// The directory holding the image, `.oth` or `.rti`. Relative host
    /// paths (a `FilesRoot`) resolve against it.
    pub dir: PathBuf,
    /// Notes from building, for `build -v`: each path an include replaced.
    pub log: Vec<String>,
}

/// What a sealed file holds besides the blobs.
#[derive(Serialize, Deserialize)]
struct Manifest {
    program: Program,
    modules: BTreeMap<String, Hash>,
    sources: BTreeMap<String, Hash>,
    trees: BTreeMap<String, Tree>,
    /// Each blob's offset from the start of the blobs, and its length.
    blobs: BTreeMap<Hash, (u64, u64)>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BuildOptions {
    /// Leave the reference sources out.
    pub no_sources: bool,
}

impl Pack {
    pub fn blob(&self, hash: &str) -> &[u8] {
        &self.blobs[hash]
    }

    /// Stores `bytes`, returning its hash.
    fn add(&mut self, bytes: Vec<u8>) -> Hash {
        add(&mut self.blobs, bytes)
    }

    /// Writes the pack as a sealed `.rti` file.
    pub fn seal(&self) -> Vec<u8> {
        let mut index = BTreeMap::new();
        let mut offset = 0u64;
        for (hash, bytes) in &self.blobs {
            index.insert(hash.clone(), (offset, bytes.len() as u64));
            offset += bytes.len() as u64;
        }
        let manifest = Manifest {
            program: self.program.clone(),
            modules: self.modules.clone(),
            sources: self.sources.clone(),
            trees: self.trees.clone(),
            blobs: index,
        };
        let doc = bson::serialize_to_document(&manifest).expect("a manifest always serializes");
        let mut manifest = Vec::new();
        doc.to_writer(&mut manifest).expect("writing BSON to a Vec cannot fail");

        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
        out.extend_from_slice(&manifest);
        for bytes in self.blobs.values() {
            out.extend_from_slice(bytes);
        }
        out
    }

    /// Writes every module, source and packed file under `out`: in
    /// `modules/`, `sources/` and `files/<instance>/`.
    pub fn unpack(&self, out: &Path) -> Result<()> {
        let mut files: Vec<(PathBuf, &str)> = vec![];
        for (type_name, hash) in &self.modules {
            files.push((Path::new("modules").join(format!("{type_name}.wasm")), hash));
        }
        for (rel, hash) in &self.sources {
            files.push((Path::new("sources").join(rel), hash));
        }
        for (name, tree) in &self.trees {
            let root = Path::new("files").join(name);
            for (path, node) in tree.nodes() {
                let rel = root.join(path.trim_start_matches('/'));
                match node {
                    Node::Dir => {
                        let dir = out.join(&rel);
                        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?
                    }
                    Node::File { hash, .. } => files.push((rel, hash)),
                }
            }
        }
        for (rel, hash) in files {
            let path = out.join(&rel);
            std::fs::create_dir_all(path.parent().unwrap()).with_context(|| format!("creating {}", path.display()))?;
            std::fs::write(&path, self.blob(hash)).with_context(|| format!("writing {}", path.display()))?;
        }
        Ok(())
    }
}

/// Stores `bytes` in `blobs`, once, returning its hash.
pub(crate) fn add(blobs: &mut BTreeMap<Hash, Rc<[u8]>>, bytes: Vec<u8>) -> Hash {
    let hash = hash(&bytes);
    blobs.entry(hash.clone()).or_insert_with(|| bytes.into());
    hash
}

fn hash(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// [`check::describe`], after each slot's value and where its fields came
/// from, and what the image takes from the host: each of its needs, and
/// what it touches there. Each object in memory says what it holds. A
/// structured slot's fields are shown by its format, one of `formats`.
pub fn describe(pack: &Pack, formats: &Formats) -> String {
    let p = &pack.program;
    let mut out = String::new();
    for slot in &p.slots {
        out.push_str(&describe_slot(slot, formats));
        out.push('\n');
    }
    if !p.slots.is_empty() {
        out.push('\n');
    }
    let config = |inst: &check::Instance| inst.config.as_ref().and_then(|c| c.text().ok()).unwrap_or_else(|| "?".into());
    let host = |i: usize| -> Option<String> {
        let inst = &p.instances[i];
        Some(match &inst.kind {
            Kind::Console => "reads stdin, writes stdout".to_string(),
            Kind::Tcp => format!("listens on {}", config(inst)),
            Kind::Files => format!("reads host files under {}", config(inst)),
            _ => return None,
        })
    };
    let needs: Vec<usize> = (0..p.instances.len()).filter(|&i| host(i).is_some()).collect();
    if !needs.is_empty() {
        out.push_str("from the host:\n");
        let width = needs.iter().map(|&i| p.instances[i].name.len()).max().unwrap_or(0);
        for &i in &needs {
            out.push_str(&format!("  {:width$}  {}\n", p.instances[i].name, host(i).unwrap()));
        }
        out.push('\n');
    }
    let about = |i: usize| {
        let inst = &p.instances[i];
        Some(match &inst.kind {
            Kind::MemoryConsole => "a console in memory".to_string(),
            Kind::MemoryListener => "listens in memory, for /dial".to_string(),
            Kind::Packed { .. } => {
                let (files, size) = pack.trees.get(&inst.name)?.size();
                format!("serves {files} built-in file{}, {}", if files == 1 { "" } else { "s" }, human_size(size))
            }
            _ => return None,
        })
    };
    out.push_str(&check::describe(p, &about, &|i| host(i).is_some()));
    out
}

/// `slot listen : text/plain = "0.0.0.0:80" (--set)`, or for a structured
/// slot, each field's value and where it came from.
fn describe_slot(slot: &check::Slot, formats: &Formats) -> String {
    let ty = &slot.ty;
    let origin = |f: &str| slot.origins.iter().find(|(name, _)| name == f).map(|(_, o)| o.as_str()).unwrap_or("?");
    let Some(value) = &slot.value else { return format!("slot {} : {ty} (unbound)", slot.name) };
    if let Some(f) = formats.for_type(ty) {
        let fields = value.payload().ok().and_then(|p| f.show(&p.bytes));
        let Some(fields) = fields else { return format!("slot {} : {ty} (unreadable)", slot.name) };
        let shown: Vec<String> = fields.iter().map(|(f, v)| format!("{f}: {v} ({})", origin(f))).collect();
        return format!("slot {} : {ty} {{ {} }}", slot.name, shown.join(", "));
    }
    let text = value.text().unwrap_or_default();
    format!("slot {} : {ty} = {} ({})", slot.name, render::json_string(&text), origin(""))
}

fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// Whether `bytes` start like a sealed image, of any format version.
pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.len() >= MAGIC.len() && bytes.starts_with(&MAGIC[..7])
}

/// Whether the file at `path` is a sealed image, going by its first bytes.
pub fn is_sealed_file(path: &Path) -> bool {
    let mut head = [0; MAGIC.len()];
    std::fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok() && is_sealed(&head)
}

/// Opens a sealed image if `path` is one, and otherwise builds the `.oth`
/// at `path` in memory.
pub fn load(path: &Path) -> Result<Pack> {
    if is_sealed_file(path) { open(path) } else { build(path, BuildOptions::default()) }
}

/// Opens the sealed image at `path`.
pub fn open(path: &Path) -> Result<Pack> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    unseal(&bytes, dir).with_context(|| format!("opening {}", path.display()))
}

/// Reads a sealed image from `bytes`, checking every blob against its hash.
pub fn unseal(bytes: &[u8], dir: PathBuf) -> Result<Pack> {
    if !is_sealed(bytes) {
        bail!("not a built image (bad magic number)");
    }
    let rebuild = "rebuild it from its .oth with this othismo's `build`";
    if bytes[7] != VERSION {
        bail!("it was built by another version of othismo (format {}, and this one reads {VERSION}); {rebuild}", bytes[7]);
    }
    let rest = &bytes[MAGIC.len()..];
    let Some((len, rest)) = rest.split_first_chunk::<8>() else { bail!("truncated header") };
    let len = usize::try_from(u64::from_le_bytes(*len)).ok().filter(|&n| n <= rest.len());
    let Some(len) = len else { bail!("truncated manifest") };
    let (manifest, blobs) = rest.split_at(len);
    let malformed = || format!("malformed manifest; if it was built by another version of othismo, {rebuild}");
    let doc = bson::Document::from_reader(manifest).with_context(malformed)?;
    let m: Manifest = bson::deserialize_from_document(doc).with_context(malformed)?;

    let mut pack = Pack {
        program: m.program,
        modules: m.modules,
        sources: m.sources,
        trees: m.trees,
        blobs: BTreeMap::new(),
        dir,
        log: vec![],
    };
    for (h, (offset, len)) in m.blobs {
        let range = usize::try_from(offset).ok().zip(usize::try_from(len).ok()).and_then(|(o, l)| Some(o..o.checked_add(l)?));
        let Some(bytes) = range.and_then(|r| blobs.get(r)) else { bail!("blob {h} is out of bounds") };
        if hash(bytes) != h {
            bail!("blob {h} is corrupt");
        }
        pack.blobs.insert(h, bytes.into());
    }
    let files = pack.trees.values().flat_map(|t| t.nodes()).filter_map(|(_, n)| match n {
        Node::File { hash, .. } => Some(hash),
        Node::Dir => None,
    });
    let missing = pack.modules.values().chain(pack.sources.values()).chain(files).find(|h| !pack.blobs.contains_key(*h));
    if let Some(h) = missing {
        bail!("blob {h} is missing");
    }
    for inst in &pack.program.instances {
        match &inst.kind {
            Kind::Wasm { type_name, .. } if !pack.modules.contains_key(type_name) => {
                bail!("instance `{}`: module {type_name} is missing", inst.name)
            }
            Kind::Packed { .. } if !pack.trees.contains_key(&inst.name) => {
                bail!("instance `{}`: its files are missing", inst.name)
            }
            _ => {}
        }
    }
    Ok(pack)
}

/// Checks the image at `path` and reads in everything it refers to.
pub fn build(path: &Path, options: BuildOptions) -> Result<Pack> {
    if is_sealed_file(path) {
        bail!("{} is already built", path.display());
    }
    let program = check::load(path)?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut pack = Pack {
        program,
        modules: BTreeMap::new(),
        sources: BTreeMap::new(),
        trees: BTreeMap::new(),
        blobs: BTreeMap::new(),
        dir: dir.clone(),
        log: vec![],
    };

    for inst in &pack.program.instances {
        let Kind::Packed { includes } = &inst.kind else { continue };
        let mut tree = Tree::default();
        for inc in includes {
            let replaced = tree
                .include(&dir.join(&inst.dir), inc, &mut |bytes| add(&mut pack.blobs, bytes))
                .with_context(|| format!("instance `{}`: include \"{}\"", inst.name, inc.source))?;
            for path in replaced {
                pack.log.push(format!("{}: include \"{}\" replaced {path}", inst.name, inc.source));
            }
        }
        pack.trees.insert(inst.name.clone(), tree);
    }

    let mut sources = pack.program.sources.clone();
    for inst in pack.program.instances.clone() {
        let Kind::Wasm { type_name, module } = &inst.kind else { continue };
        if pack.modules.contains_key(type_name) {
            continue;
        }
        let module = dir.join(module);
        let bytes = std::fs::read(&module).with_context(|| {
            format!("instance `{}`: cannot read {} (did you compile {type_name}.ts?)", inst.name, module.display())
        })?;
        let hash = pack.add(bytes);
        pack.modules.insert(type_name.clone(), hash);
        sources.push(module.with_extension("oth"));
        sources.push(module.with_extension("ts"));
    }

    if !options.no_sources {
        for path in sources {
            // A module's header and TypeScript are optional; the image and its imports were just read.
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let hash = pack.add(bytes);
            pack.sources.insert(source_name(&dir, &path), hash);
        }
    }
    Ok(pack)
}

/// A source's name in the pack: its path relative to the image, with `..`
/// spelled `__` so that unpacking never leaves the output directory.
fn source_name(dir: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(dir).unwrap_or(path);
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
            Component::ParentDir => Some("__".into()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_stay_inside() {
        let dir = Path::new("/img");
        assert_eq!(source_name(dir, Path::new("/img/scripts/A.ts")), "scripts/A.ts");
        assert_eq!(source_name(dir, Path::new("/img/../shared/m.ts")), "__/shared/m.ts");
        assert_eq!(source_name(dir, Path::new("/elsewhere/m.ts")), "elsewhere/m.ts");
    }
}
