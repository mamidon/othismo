//! What every part of Othismo shares, and what doesn't change when a
//! module, a message type or a payload format is added: the namespace
//! (Layer 0), the protocol's verbs, errors and payloads (Layer 1), and the
//! path patterns headers write. See `docs/protocol.md`.

pub mod ns;
pub mod pattern;
pub mod payload;
