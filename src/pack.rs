//! Built images: [`image::pack`], and building one with this othismo's
//! formats.

pub use ::compiler::build::BuildOptions;
pub use ::image::pack::*;
use anyhow::Result;
use std::path::Path;

/// Opens a sealed image if `path` is one, and otherwise builds the `.oth`
/// at `path` in memory.
pub fn load(path: &Path) -> Result<Pack> {
    ::compiler::build::load(path, crate::formats())
}

/// Checks the image at `path` and reads in everything it refers to.
pub fn build(path: &Path, options: BuildOptions) -> Result<Pack> {
    ::compiler::build::build(path, crate::formats(), options)
}
