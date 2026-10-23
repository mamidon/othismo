//! A built image, as the runtime runs it: the interfaces of the objects in
//! it, the checked [`program::Program`], and the sealed `.rti` file that
//! carries it all. The compiler makes one; nothing here reads `.oth`
//! files.

pub mod header;
pub mod pack;
pub mod program;
pub mod slots;
pub mod tree;
