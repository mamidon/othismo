//! The local operator's hold on a running image: what `othismo shell` uses
//! to look at an image and make requests of it while it runs.
//!
//! The operator is whoever runs the image, at the terminal it runs on.
//! Commands come in over an in-process channel, from an [`Operator`] that
//! only the code that started the runtime can hold: there is no socket, and
//! nothing outside the process can send one. Reaching a running image from
//! elsewhere would have to be opted into, and isn't offered.
//!
//! A request is made from a point of view: the root image's view, or an
//! instance's. It is resolved in that view and made as that instance would
//! make it. From an image (the root, or an instance of one) it carries the
//! operator flag, as an image's startup requests do; from any other
//! instance it doesn't, since objects never can, and it is held to that
//! instance's needs like the instance's own requests.

use crate::env::Incoming;
use crate::runtime::Answered;
use format::AllSchemas;
use image::header::Object;
use image::program::Kind;
use protocol::{namespace::Resolved, path::MountPoint, verb::Verb};
use protocol::payload::Payload;
use std::sync::mpsc::{self, Receiver, Sender};

/// What the operator asks of a running image.
#[derive(Debug)]
pub enum Command {
    /// Makes `verb path [body]` from `from`'s point of view: an instance,
    /// or `None` for the root image. Its answers come back as
    /// [`Notice::Answer`]s carrying `id`, the last one marked done.
    Request { id: u64, from: Option<usize>, verb: Verb, path: String, body: Option<Payload> },
    /// Asks instance `inst` itself, as the operator, about `path` in the
    /// tree it serves, as [`crate::Runtime::ask`] does: how the shell sees
    /// inside served trees, whoever it is looking from. Only a safe verb
    /// can be sent this way: looking changes nothing. Answered as a request.
    Look { id: u64, inst: usize, verb: Verb, path: String },
    /// Gives up on request `id`. It is answered `ok done` straight away,
    /// and anything its server says after that is dropped.
    Cancel { id: u64 },
    /// Asks for a [`Notice::Snapshot`] of the image as it is now.
    Snapshot,
    /// Ends the run once what is queued has been delivered.
    Stop,
}

/// What a running image tells the operator.
#[derive(Debug)]
pub enum Notice {
    Answer { id: u64, answer: Answered },
    Snapshot(Snapshot),
}

/// The operator's end: commands sent here reach the runtime's loop.
#[derive(Clone)]
pub struct Operator(Sender<Incoming>);

impl Operator {
    /// Sends `command`; false once the run has ended.
    pub fn send(&self, command: Command) -> bool {
        self.0.send(Incoming::Command(command)).is_ok()
    }
}

/// The runtime's end, which [`crate::Runtime::run_operated`] takes.
pub struct Inbox {
    pub(crate) tx: Sender<Incoming>,
    pub(crate) rx: Receiver<Incoming>,
}

/// A connected pair: keep the [`Operator`], and give the [`Inbox`] to the
/// runtime that it should operate.
pub fn channel() -> (Operator, Inbox) {
    let (tx, rx) = mpsc::channel();
    (Operator(tx.clone()), Inbox { tx, rx })
}

/// A running image as it is at one moment: every instance and every view,
/// including what has changed since it started.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// In the image's order; [`Link::target`]s refer to them by index.
    pub instances: Vec<InstanceInfo>,
    /// The root image's own view.
    pub root: Vec<Link>,
    /// Each format's schemas, by content type, for reading and writing
    /// payloads.
    pub messages: AllSchemas,
}

#[derive(Debug, Clone)]
pub struct InstanceInfo {
    /// `web.http`; one of the root image's needs is named by its mount
    /// point, `/net/tcp`.
    pub name: String,
    pub type_name: String,
    pub kind: Kind,
    /// What it serves and needs.
    pub interface: Object,
    /// Its view as it is now.
    pub view: Vec<Link>,
    /// What it has spliced into its served tree: an image's exports, or an
    /// object's `expose`s.
    pub exposed: Vec<Link>,
    /// Requests it has been sent.
    pub served: u32,
    /// Requests it has been sent and hasn't finished answering.
    pub pending: usize,
}

/// A mount point in a view, or an exposure in a served tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub at: MountPoint,
    pub target: Resolved,
    pub how: How,
}

/// How a link came to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// The image mounted the target here: this view holds it.
    Mount,
    /// The image bound something it already had here, or exported it.
    Bind,
    /// The instance made it while running, with `bind` or `expose`.
    Runtime,
}
