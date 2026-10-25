//! A runtime for images: declarative graphs of sandboxed AssemblyScript
//! objects that reach each other only through the paths in their namespaces.
//!
//! This is the `othismo` command's library: the workspace's crates, under
//! the names the command and its tests use, and the formats it knows.

pub mod pack;

pub use ::codegen::{bindings, doc};
pub use ::compiler::{check, iface};
pub use ::image::{slots, tree};
pub use ::shell;
pub use ::runtime::{env, operator, render, runtime, suite};
pub use format_bson::{messages, wire};
pub use protocol::{namespace, path, pattern, payload, status, verb};
pub use syntax::image_definition;

/// The formats this othismo knows: BSON's.
pub fn formats() -> format::Formats {
    format::Formats::new(vec![Box::new(format_bson::Bson)])
}

/// The AssemblyScript bindings for the `.oth` file at `source`, as
/// `othismo bindings` writes them, importing the SDK from `library`: its
/// server bases and clients, and the classes for its schemas' types.
/// `None` if it has nothing to bind.
pub fn bindings_for(source: &std::path::Path, library: &str) -> anyhow::Result<Option<String>> {
    let formats = formats();
    let mut loader = iface::Loader::new(formats.clone());
    let header = loader.header(source)?;
    // Its own types, and those of each file it imports, which its
    // objects may name.
    let dir = source.parent().unwrap_or(std::path::Path::new("."));
    let definition = loader.image_definition(source)?;
    let mut schemas = vec![];
    for f in formats.iter() {
        let mut all = loader.schemas(f, source)?;
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut from = f.declared(&all).into_iter().map(|t| (t, name.clone())).collect();
        for imp in &definition.header.imports {
            if let image_definition::Source::Path(p) = &imp.from {
                let theirs = loader.schemas(f, &iface::join(dir, p))?;
                f.merge(&mut all, &mut from, &theirs, p)?;
            }
        }
        schemas.push((f, all));
    }
    let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(bindings::namespace(&header, &schemas, &name, library, "othismo/env"))
}
