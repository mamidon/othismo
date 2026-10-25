//! What a request asks for, and the flags it is made with.

use serde::{Deserialize, Serialize};
use std::fmt;

/// What a request asks for. Safe verbs never change anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verb {
    Read,
    Stat,
    List,
    Watch,
    Write,
    Create,
    Remove,
}

pub const VERBS: [Verb; 7] = [
    Verb::Read,
    Verb::Stat,
    Verb::List,
    Verb::Watch,
    Verb::Write,
    Verb::Create,
    Verb::Remove,
];

impl Verb {
    pub fn from_i32(v: i32) -> Option<Verb> {
        VERBS.get(usize::try_from(v).ok()?).copied()
    }

    pub fn is_safe(self) -> bool {
        matches!(self, Verb::Read | Verb::Stat | Verb::List | Verb::Watch)
    }

    /// The verb's name as written in images: `read`, `write`, ...
    pub fn keyword(self) -> &'static str {
        match self {
            Verb::Read => "read",
            Verb::Stat => "stat",
            Verb::List => "list",
            Verb::Watch => "watch",
            Verb::Write => "write",
            Verb::Create => "create",
            Verb::Remove => "remove",
        }
    }

    pub fn from_keyword(s: &str) -> Option<Verb> {
        VERBS.into_iter().find(|v| v.keyword() == s)
    }
}

impl fmt::Display for Verb {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// `Flag.ReadOnly` in the SDK: only safe verbs pass.
pub const READ_ONLY: i32 = 1;

/// A request's flag saying whoever runs the image made it: its startup
/// requests, and those an image makes of what it instantiates. No object
/// can set it.
pub const OPERATOR: i32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_round_trip() {
        for (i, v) in VERBS.iter().enumerate() {
            assert_eq!(Verb::from_i32(i as i32), Some(*v));
            assert_eq!(Verb::from_keyword(v.keyword()), Some(*v));
        }
        assert_eq!(Verb::from_i32(7), None);
        assert!(Verb::Watch.is_safe() && !Verb::Remove.is_safe() && !Verb::Create.is_safe());
    }
}
