//! How a request went.

use std::borrow::Cow;
use std::fmt;

/// The outcome carried by every answer: ok, or an error (see
/// `docs/protocol.md`).
///
/// An error's errno is what a program compares, and its code names it. Both
/// are up to whoever answers: any errno but 0 is an error, and an instance
/// should give the same errno the same code every time. The statuses the
/// runtime answers with itself, and that servers may reuse, are below.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// 0 for ok.
    pub errno: i32,
    /// `snake_case`, such as `not_found`.
    pub code: Cow<'static, str>,
}

impl Status {
    pub const OK: Status = Status::known(0, "ok");
    /// Nothing is at the path, or the sender's view doesn't reach it.
    pub const NOT_FOUND: Status = Status::known(1, "not_found");
    /// An unsafe verb through a read-only mount, or of something that can't change.
    pub const READ_ONLY: Status = Status::known(2, "read_only");
    /// Something is at the path, but it doesn't take that verb.
    pub const NOT_SUPPORTED: Status = Status::known(3, "not_supported");
    /// A directory where something else was needed.
    pub const IS_DIRECTORY: Status = Status::known(4, "is_directory");
    /// Something else where a directory was needed.
    pub const NOT_DIRECTORY: Status = Status::known(5, "not_directory");
    /// `create` of something already there.
    pub const EXISTS: Status = Status::known(6, "exists");
    /// A malformed path, or a payload the server couldn't make sense of.
    pub const INVALID: Status = Status::known(7, "invalid");
    /// The outside world failed.
    pub const IO: Status = Status::known(8, "io");
    /// A request only the operator may make, from an object.
    pub const FORBIDDEN: Status = Status::known(9, "forbidden");
    /// The server doesn't take payloads of that content type.
    pub const UNSUPPORTED_TYPE: Status = Status::known(10, "unsupported_type");

    const fn known(errno: i32, code: &'static str) -> Status {
        Status { errno, code: Cow::Borrowed(code) }
    }

    /// An error a server answers with, if `errno` isn't 0 and `code` is
    /// `snake_case`.
    pub fn error(errno: i32, code: impl Into<Cow<'static, str>>) -> Result<Status, String> {
        let code = code.into();
        if errno == 0 {
            return Err(format!("errno 0 is ok, not an error (code `{code}`)"));
        }
        let snake_case = code.starts_with(|c: char| c.is_ascii_lowercase())
            && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !snake_case {
            return Err(format!("errno {errno}'s code `{code}` isn't snake_case"));
        }
        Ok(Status { errno, code })
    }

    pub fn is_ok(&self) -> bool {
        self.errno == 0
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_choose_their_own_errors() {
        assert_eq!(Status::error(42, "over_quota"), Ok(Status { errno: 42, code: "over_quota".into() }));
        assert_eq!(Status::error(1, "not_found"), Ok(Status::NOT_FOUND));
        assert!(Status::error(0, "ok").is_err(), "0 is ok");
        for bad in ["", "NotFound", "not-found", "_x", "9lives"] {
            assert!(Status::error(1, bad).is_err(), "{bad}");
        }
        assert!(Status::OK.is_ok() && !Status::IO.is_ok());
    }
}
