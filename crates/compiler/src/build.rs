//! Building a pack: checking an image, then reading in the modules, packed
//! files and sources it refers to. A pack itself, and sealing and opening
//! one, are the `image` crate's.

use crate::check::{self, Kind};
use format::Formats;
use ::image::pack::{Pack, add, is_sealed_file, open};
use ::image::tree::Tree;
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Component, Path};

#[derive(Debug, Clone, Copy, Default)]
pub struct BuildOptions {
    /// Leave the reference sources out.
    pub no_sources: bool,
}

/// Opens a sealed image if `path` is one, and otherwise builds the `.oth`
/// at `path` in memory.
pub fn load(path: &Path, formats: Formats) -> Result<Pack> {
    if is_sealed_file(path) { open(path) } else { build(path, formats, BuildOptions::default()) }
}

/// Checks the image at `path` and reads in everything it refers to.
pub fn build(path: &Path, formats: Formats, options: BuildOptions) -> Result<Pack> {
    if is_sealed_file(path) {
        bail!("{} is already built", path.display());
    }
    let program = check::load(path, formats)?;
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
