//! A runtime for images: declarative graphs of sandboxed AssemblyScript
//! objects that reach each other only through the paths in their namespaces.
//!
//! This is the `othismo` command's library: the workspace's crates, under
//! the names the command and its tests use, and the formats it knows.

pub mod pack;

pub use ::codegen::{bindings, doc};
pub use ::compiler::{check, iface};
pub use ::image::{slots, tree};
pub use ::runtime::{env, render, runtime, suite};
pub use format_bson::{messages, wire};
pub use protocol::{ns, pattern, payload};
pub use syntax::image_definition;

/// The formats this othismo knows: BSON's.
pub fn formats() -> format::Formats {
    format::Formats::new(vec![Box::new(format_bson::Bson)])
}
