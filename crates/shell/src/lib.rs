//! `othismo shell`: the local operator's interactive shell.
//!
//! It runs an image in this process and lets whoever runs it look around
//! the running image from any point of view and make requests of it. Only
//! the person at this terminal can: the image is reached through an
//! in-process channel, never a socket (see `runtime::operator`).
//!
//! Every display has a point of view. At first it is the image itself,
//! which only lists the roots to look from: its own view, and every
//! instance nothing mounts. From a root, the operator steps into the views
//! of what it holds; the route back to the root is always shown.

pub mod command;
pub mod repl;
pub mod session;
pub mod show;
pub mod world;

pub use session::{Event, Outcome, Session};
pub use world::{Viewpoint, World};
