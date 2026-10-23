//! A runtime for images: declarative graphs of sandboxed AssemblyScript
//! objects that reach each other only through the paths in their namespaces.

pub mod bindings;
pub mod check;
pub mod doc;
pub mod env;
pub mod iface;
pub mod pack;
pub mod render;
pub mod runtime;
pub mod suite;

pub use protocol::{ns, pattern, payload};
pub use format_bson::{messages, wire};

/// The formats this othismo knows: BSON's.
pub fn formats() -> format::Formats {
    format::Formats::new(vec![Box::new(format_bson::Bson)])
}
pub use ::image::{slots, tree};
pub use syntax::image;
