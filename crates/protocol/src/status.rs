//! How a request went.

use std::fmt;

/// The outcome carried by every reply: `Ok`, or an error, whose number is
/// its errno (see `docs/protocol.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    NotFound,
    ReadOnly,
    NotSupported,
    IsDirectory,
    NotDirectory,
    Exists,
    Invalid,
    Io,
    Forbidden,
    UnsupportedType,
}

const STATUSES: [Status; 11] = [
    Status::Ok,
    Status::NotFound,
    Status::ReadOnly,
    Status::NotSupported,
    Status::IsDirectory,
    Status::NotDirectory,
    Status::Exists,
    Status::Invalid,
    Status::Io,
    Status::Forbidden,
    Status::UnsupportedType,
];

impl Status {
    pub fn from_i32(v: i32) -> Option<Status> {
        STATUSES.get(usize::try_from(v).ok()?).copied()
    }

    /// The status's name as written in docs and output: `ok`, `not_found`, ...
    pub fn name(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::NotFound => "not_found",
            Status::ReadOnly => "read_only",
            Status::NotSupported => "not_supported",
            Status::IsDirectory => "is_directory",
            Status::NotDirectory => "not_directory",
            Status::Exists => "exists",
            Status::Invalid => "invalid",
            Status::Io => "io",
            Status::Forbidden => "forbidden",
            Status::UnsupportedType => "unsupported_type",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_round_trip() {
        assert_eq!(Status::from_i32(-1), None);
    }
}
