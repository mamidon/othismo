//! A built image: a checked [`Program`] plus everything it needs from disk,
//! held in memory. The runtime only ever runs a `Pack`.

use crate::check::{self, Kind, Program};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Pack {
    pub program: Program,
    /// Compiled modules by instance type.
    pub modules: BTreeMap<String, Vec<u8>>,
}

/// Checks the image at `path` and reads in everything it refers to.
pub fn build(path: &Path) -> Result<Pack> {
    let program = check::load(path)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut modules = BTreeMap::new();
    for inst in &program.instances {
        if let Kind::Wasm { type_name } = &inst.kind
            && !modules.contains_key(type_name)
        {
            let module = module_path(dir, type_name);
            let bytes = std::fs::read(&module).with_context(|| {
                format!("instance `{}`: cannot read {} (did you compile {type_name}.ts?)", inst.name, module.display())
            })?;
            modules.insert(type_name.clone(), bytes);
        }
    }
    Ok(Pack { program, modules })
}

/// Where instance type `type_name` is compiled to, next to the image.
pub fn module_path(dir: &Path, type_name: &str) -> PathBuf {
    dir.join("scripts").join(format!("{type_name}.wasm"))
}
