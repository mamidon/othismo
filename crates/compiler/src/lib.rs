//! The compiler: reads `.oth` files and the headers and schemas inside
//! modules, resolves every name they use, checks an image and everything it
//! instantiates, and builds the [`image::pack::Pack`] the runtime runs.
//! It encodes payload literals with whichever formats it is given, and
//! knows none itself.

pub mod build;
pub mod check;
pub mod iface;
