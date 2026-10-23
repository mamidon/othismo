//! The runtime: runs a built image's objects, each module sandboxed in
//! wasmtime, the host's objects natively, and routes every request between
//! them through the namespace. It runs a [`image::pack::Pack`] and reads
//! no `.oth` files and no payloads.

pub mod env;
pub mod render;
pub mod runtime;
pub mod suite;

pub use runtime::*;
