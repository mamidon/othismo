//! A runtime for images: declarative graphs of sandboxed AssemblyScript
//! objects that reach each other only through the paths in their namespaces.

pub mod bindings;
pub mod check;
pub mod doc;
pub mod env;
pub mod iface;
pub mod image;
pub mod messages;
pub mod ns;
pub mod pack;
pub mod pattern;
pub mod render;
pub mod runtime;
pub mod slots;
pub mod tree;
pub mod wire;
