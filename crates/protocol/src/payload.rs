//! Payloads: what a request or an answer carries besides its verb, path and
//! status. A payload is a content type and some bytes, which the runtime
//! carries without looking inside (see `docs/protocol.md`). The one layout
//! the protocol defines itself is the stat record, which `stat` and `list`
//! answer with.

use serde::{Deserialize, Serialize};

/// Text, in UTF-8.
pub const TEXT: &str = "text/plain";
/// Bytes with no more particular type.
pub const OCTETS: &str = "application/octet-stream";
/// A stat record, which headers can write as `Stat`.
pub const STAT: &str = "application/x-othismo-stat";
/// What headers write for a stat record's content type.
pub const STAT_NAME: &str = "Stat";
/// Any payload at all.
pub const ANY: &str = "*/*";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    /// e.g. `text/plain`.
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl Payload {
    pub fn new(content_type: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Payload {
        Payload { content_type: content_type.into(), bytes: bytes.into() }
    }

    /// `text/plain`.
    pub fn text(text: &str) -> Payload {
        Payload::new(TEXT, text.as_bytes())
    }

    /// Whether it is text of any kind, `text/*`.
    pub fn is_text(&self) -> bool {
        fits("text/*", &self.content_type)
    }
}

/// What `stat` answers: one entry in a served tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    /// The last segment of its path.
    pub name: String,
    pub is_directory: bool,
    /// In bytes, or 0 for a directory or if it isn't known.
    pub length: u64,
    /// The content type a `read` of the entry would answer with, or empty.
    /// Not the record's own, which is always [`STAT`]: this is how a
    /// listing says what each entry holds without reading it.
    pub content_type: String,
    /// Changes whenever what a `read` would answer changes, or 0.
    pub version: u64,
}

impl Stat {
    pub fn file(name: &str, length: u64, content_type: &str) -> Stat {
        Stat { name: name.into(), is_directory: false, length, content_type: content_type.into(), version: 0 }
    }

    pub fn directory(name: &str) -> Stat {
        Stat { name: name.into(), is_directory: true, length: 0, content_type: String::new(), version: 0 }
    }

    /// `name[s] kind[1] length[8] type[s] version[8]`, little-endian, where
    /// `[s]` is a string with a 2-byte length first.
    pub fn encode(&self) -> Payload {
        let mut bytes = vec![];
        Stat::put_string(&mut bytes, &self.name);
        bytes.push(self.is_directory as u8);
        bytes.extend_from_slice(&self.length.to_le_bytes());
        Stat::put_string(&mut bytes, &self.content_type);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        Payload::new(STAT, bytes)
    }

    fn put_string(bytes: &mut Vec<u8>, string: &str) {
        let length = u16::try_from(string.len()).expect("a stat record's strings are short");
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(string.as_bytes());
    }

    /// The stat record in `payload`, if it is a well-formed one.
    pub fn decode(payload: &Payload) -> Option<Stat> {
        if payload.content_type != STAT {
            return None;
        }
        let mut reader = Reader(&payload.bytes);
        let name = reader.take_string()?;
        let is_directory = match reader.take(1)? {
            [0] => false,
            [1] => true,
            _ => return None,
        };
        let length = reader.take_u64()?;
        let content_type = reader.take_string()?;
        let version = reader.take_u64()?;
        reader.0.is_empty().then_some(Stat { name, is_directory, length, content_type, version })
    }
}

/// What is left of a stat record being decoded.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    /// The next `count` bytes.
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.0.len() < count {
            return None;
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Some(head)
    }

    fn take_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn take_string(&mut self) -> Option<String> {
        let length = u16::from_le_bytes(self.take(2)?.try_into().ok()?);
        String::from_utf8(self.take(length as usize)?.to_vec()).ok()
    }
}

/// A content type taken apart: its type and subtype, lowercase, and its
/// parameters, with lowercase names.
struct ParsedContentType {
    type_name: String,
    subtype: String,
    parameters: Vec<(String, String)>,
}

fn parse(content_type: &str) -> Option<ParsedContentType> {
    let mut parts = content_type.split(';');
    let (type_name, subtype) = parts.next()?.trim().split_once('/')?;
    let token = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "!#$&^_.+-".contains(c));
    let well_formed = (type_name == "*" && subtype == "*") || (token(type_name) && (subtype == "*" || token(subtype)));
    if !well_formed {
        return None;
    }
    let mut parameters = vec![];
    for parameter in parts {
        let (name, value) = parameter.trim().split_once('=')?;
        if !token(name.trim()) || value.trim().is_empty() {
            return None;
        }
        parameters.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Some(ParsedContentType {
        type_name: type_name.to_ascii_lowercase(),
        subtype: subtype.to_ascii_lowercase(),
        parameters,
    })
}

/// Whether `content_type` is well-formed: `type/subtype`, `type/*` or
/// `*/*`, optionally followed by `; name=value` parameters.
pub fn valid(content_type: &str) -> bool {
    parse(content_type).is_some()
}

/// Whether a payload of content type `given` fits where `want` is written.
/// `*/*` takes anything and `text/*` any text; a parameter `want` writes
/// must be in `given` with the same value. A wildcard fits only an equal or
/// wider wildcard: something that may be any text doesn't fit `text/plain`.
pub fn fits(want: &str, given: &str) -> bool {
    let (Some(want), Some(given)) = (parse(want), parse(given)) else { return false };
    let types_fit = want.type_name == "*"
        || (want.type_name == given.type_name && (want.subtype == "*" || want.subtype == given.subtype));
    types_fit && want.parameters.iter().all(|parameter| given.parameters.contains(parameter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_fit_by_wildcards_and_parameters() {
        assert!(fits("*/*", "text/plain"));
        assert!(fits("*/*", "*/*"));
        assert!(fits("text/*", "text/html; charset=utf-8"));
        assert!(fits("text/plain", "TEXT/Plain"));
        assert!(fits("text/html", "text/html; charset=utf-8"), "a parameter the want leaves out constrains nothing");
        assert!(!fits("text/html; charset=utf-8", "text/html"));
        assert!(!fits("text/plain", "text/*"), "maybe any text isn't plain text");
        assert!(!fits("text/*", "*/*"));
        assert!(!fits("text/plain", "application/bson"));
        assert!(!fits("text/plain", "nonsense"));
    }

    #[test]
    fn only_well_formed_content_types_are_valid() {
        for ok in ["text/plain", "application/x-othismo-stat", "text/*", "*/*", "text/html; charset=utf-8", "image/svg+xml"] {
            assert!(valid(ok), "{ok}");
        }
        for bad in ["text", "*/plain", "text/", "/plain", "text/plain;", "text/plain; charset", "te xt/plain"] {
            assert!(!valid(bad), "{bad}");
        }
    }

    #[test]
    fn stat_records_round_trip() {
        let stat = Stat { name: "a.txt".into(), is_directory: false, length: 12, content_type: "text/plain".into(), version: 7 };
        let payload = stat.encode();
        assert_eq!(payload.content_type, STAT);
        assert_eq!(payload.bytes.len(), 2 + 5 + 1 + 8 + 2 + 10 + 8);
        assert_eq!(Stat::decode(&payload), Some(stat));
        assert_eq!(Stat::decode(&Stat::directory("sub").encode()).unwrap().is_directory, true);
        assert_eq!(Stat::decode(&Payload::new(STAT, vec![1])), None);
        assert_eq!(Stat::decode(&Payload::text("hi")), None);
        let mut long = Stat::directory("x").encode();
        long.bytes.push(0);
        assert_eq!(Stat::decode(&long), None, "nothing may follow");
    }
}
