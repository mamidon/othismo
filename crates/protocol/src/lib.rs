//! What every part of Othismo shares, and what doesn't change when a
//! module, a message type or a payload format is added: the namespace
//! (Layer 0), the protocol's verbs, statuses and payloads (Layer 1), and the
//! path patterns headers write. See `docs/protocol.md`.

pub mod content_type;
pub mod namespace;
pub mod path;
pub mod pattern;
pub mod payload;
pub mod status;
pub mod verb;
