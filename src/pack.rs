//! A built image: a checked [`Program`] plus everything it needs from disk,
//! held in memory. The runtime only ever runs a `Pack`.
//!
//! A pack comes from building a `.img` ([`build`]) or from opening a sealed
//! `.rti` file ([`open`]). The file is a magic number, a BSON manifest and
//! the blobs it refers to (see `docs/images.md`):
//!
//! ```text
//! "RTIMAGE" 0x01   u64 LE manifest length   manifest   blobs
//! ```
//!
//! Blobs are named by the SHA-256 of their contents and stored once.

use crate::check::{self, Kind, Program};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub const MAGIC: &[u8; 8] = b"RTIMAGE\x01";

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
    pub blobs: BTreeMap<Hash, Vec<u8>>,
    /// The directory holding the image, `.img` or `.rti`. Relative host
    /// paths (a `FilesRoot`) resolve against it.
    pub dir: PathBuf,
}

/// What a sealed file holds besides the blobs.
#[derive(Serialize, Deserialize)]
struct Manifest {
    program: Program,
    modules: BTreeMap<String, Hash>,
    sources: BTreeMap<String, Hash>,
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
        let hash = hash(&bytes);
        self.blobs.entry(hash.clone()).or_insert(bytes);
        hash
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

    /// Writes every module, source and packed file under `out`.
    pub fn unpack(&self, out: &Path) -> Result<()> {
        let mut files: Vec<(PathBuf, &str)> = vec![];
        for (type_name, hash) in &self.modules {
            files.push((Path::new("modules").join(format!("{type_name}.wasm")), hash));
        }
        for (rel, hash) in &self.sources {
            files.push((Path::new("sources").join(rel), hash));
        }
        for (rel, hash) in files {
            let path = out.join(&rel);
            std::fs::create_dir_all(path.parent().unwrap()).with_context(|| format!("creating {}", path.display()))?;
            std::fs::write(&path, self.blob(hash)).with_context(|| format!("writing {}", path.display()))?;
        }
        Ok(())
    }
}

fn hash(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `bytes` start like a sealed image.
pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Whether the file at `path` is a sealed image, going by its first bytes.
pub fn is_sealed_file(path: &Path) -> bool {
    let mut head = [0; MAGIC.len()];
    std::fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok() && is_sealed(&head)
}

/// Opens a sealed image if `path` is one, and otherwise builds the `.img`
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
    let rest = &bytes[MAGIC.len()..];
    let Some((len, rest)) = rest.split_first_chunk::<8>() else { bail!("truncated header") };
    let len = usize::try_from(u64::from_le_bytes(*len)).ok().filter(|&n| n <= rest.len());
    let Some(len) = len else { bail!("truncated manifest") };
    let (manifest, blobs) = rest.split_at(len);
    let doc = bson::Document::from_reader(manifest).context("malformed manifest")?;
    let m: Manifest = bson::deserialize_from_document(doc).context("malformed manifest")?;

    let mut pack = Pack { program: m.program, modules: m.modules, sources: m.sources, blobs: BTreeMap::new(), dir };
    for (h, (offset, len)) in m.blobs {
        let range = usize::try_from(offset).ok().zip(usize::try_from(len).ok()).and_then(|(o, l)| Some(o..o.checked_add(l)?));
        let Some(bytes) = range.and_then(|r| blobs.get(r)) else { bail!("blob {h} is out of bounds") };
        if hash(bytes) != h {
            bail!("blob {h} is corrupt");
        }
        pack.blobs.insert(h, bytes.to_vec());
    }
    let missing = pack.modules.values().chain(pack.sources.values()).find(|h| !pack.blobs.contains_key(*h));
    if let Some(h) = missing {
        bail!("blob {h} is missing");
    }
    for inst in &pack.program.instances {
        if let Kind::Wasm { type_name } = &inst.kind
            && !pack.modules.contains_key(type_name)
        {
            bail!("instance `{}`: module {type_name} is missing", inst.name);
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
        blobs: BTreeMap::new(),
        dir: dir.clone(),
    };

    let mut sources = pack.program.sources.clone();
    for inst in pack.program.instances.clone() {
        let Kind::Wasm { type_name } = &inst.kind else { continue };
        if pack.modules.contains_key(type_name) {
            continue;
        }
        let module = module_path(&dir, type_name);
        let bytes = std::fs::read(&module).with_context(|| {
            format!("instance `{}`: cannot read {} (did you compile {type_name}.ts?)", inst.name, module.display())
        })?;
        let hash = pack.add(bytes);
        pack.modules.insert(type_name.clone(), hash);
        sources.push(module.with_extension("ts"));
    }

    if !options.no_sources {
        for path in sources {
            // A module's TypeScript is optional; the image and its imports were just read.
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let hash = pack.add(bytes);
            pack.sources.insert(source_name(&dir, &path), hash);
        }
    }
    Ok(pack)
}

/// Where instance type `type_name` is compiled to, next to the image.
pub fn module_path(dir: &Path, type_name: &str) -> PathBuf {
    dir.join("scripts").join(format!("{type_name}.wasm"))
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
