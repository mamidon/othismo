//! A runtime for images: declarative graphs of sandboxed AssemblyScript
//! objects that reach each other only through the paths in their namespaces.

pub mod bindings;
pub mod doc;
pub mod pack;

pub use ::compiler::{check, iface};
pub use ::image::{slots, tree};
pub use ::runtime::{env, render, runtime, suite};
pub use format_bson::{messages, wire};
pub use protocol::{ns, pattern, payload};
pub use syntax::image;

/// The formats this othismo knows: BSON's.
pub fn formats() -> format::Formats {
    format::Formats::new(vec![Box::new(format_bson::Bson)])
}
