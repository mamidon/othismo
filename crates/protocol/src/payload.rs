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
    /// Its content type, e.g. `text/plain`.
    pub ty: String,
    pub bytes: Vec<u8>,
}

impl Payload {
    pub fn new(ty: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Payload {
        Payload { ty: ty.into(), bytes: bytes.into() }
    }

    /// `text/plain`.
    pub fn text(s: &str) -> Payload {
        Payload::new(TEXT, s.as_bytes())
    }

    /// Whether it is text of any kind, `text/*`.
    pub fn is_text(&self) -> bool {
        fits("text/*", &self.ty)
    }
}

/// What `stat` answers: one entry in a served tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    /// The last segment of its path.
    pub name: String,
    pub dir: bool,
    /// In bytes, or 0 for a directory or if it isn't known.
    pub length: u64,
    /// The content type a `read` would answer with, or empty.
    pub ty: String,
    /// Changes whenever what a `read` would answer changes, or 0.
    pub version: u64,
}

impl Stat {
    pub fn file(name: &str, length: u64, ty: &str) -> Stat {
        Stat { name: name.into(), dir: false, length, ty: ty.into(), version: 0 }
    }

    pub fn dir(name: &str) -> Stat {
        Stat { name: name.into(), dir: true, length: 0, ty: String::new(), version: 0 }
    }

    /// `name[s] kind[1] length[8] type[s] version[8]`, little-endian, where
    /// `[s]` is a string with a 2-byte length first.
    pub fn encode(&self) -> Payload {
        let mut out = vec![];
        string(&mut out, &self.name);
        out.push(self.dir as u8);
        out.extend_from_slice(&self.length.to_le_bytes());
        string(&mut out, &self.ty);
        out.extend_from_slice(&self.version.to_le_bytes());
        Payload::new(STAT, out)
    }

    /// The stat record in `p`, if it is a well-formed one.
    pub fn decode(p: &Payload) -> Option<Stat> {
        if p.ty != STAT {
            return None;
        }
        let mut b = &p.bytes[..];
        let name = take_string(&mut b)?;
        let dir = match take(&mut b, 1)? {
            [0] => false,
            [1] => true,
            _ => return None,
        };
        let length = u64::from_le_bytes(take(&mut b, 8)?.try_into().ok()?);
        let ty = take_string(&mut b)?;
        let version = u64::from_le_bytes(take(&mut b, 8)?.try_into().ok()?);
        b.is_empty().then_some(Stat { name, dir, length, ty, version })
    }
}

fn string(out: &mut Vec<u8>, s: &str) {
    let len = u16::try_from(s.len()).expect("a stat record's strings are short");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    if b.len() < n {
        return None;
    }
    let (head, rest) = b.split_at(n);
    *b = rest;
    Some(head)
}

fn take_string(b: &mut &[u8]) -> Option<String> {
    let len = u16::from_le_bytes(take(b, 2)?.try_into().ok()?);
    String::from_utf8(take(b, len as usize)?.to_vec()).ok()
}

/// A content type taken apart: its type and subtype, lowercase, and its
/// parameters, with lowercase names.
struct Parsed {
    ty: String,
    sub: String,
    params: Vec<(String, String)>,
}

fn parse(ct: &str) -> Option<Parsed> {
    let mut parts = ct.split(';');
    let (ty, sub) = parts.next()?.trim().split_once('/')?;
    let token = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "!#$&^_.+-".contains(c));
    let ok = (ty == "*" && sub == "*") || (token(ty) && (sub == "*" || token(sub)));
    if !ok {
        return None;
    }
    let mut params = vec![];
    for p in parts {
        let (name, value) = p.trim().split_once('=')?;
        if !token(name.trim()) || value.trim().is_empty() {
            return None;
        }
        params.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Some(Parsed { ty: ty.to_ascii_lowercase(), sub: sub.to_ascii_lowercase(), params })
}

/// Whether `ct` is a well-formed content type: `type/subtype`, `type/*` or
/// `*/*`, optionally followed by `; name=value` parameters.
pub fn valid(ct: &str) -> bool {
    parse(ct).is_some()
}

/// Whether a payload of content type `given` fits where `want` is written.
/// `*/*` takes anything and `text/*` any text; a parameter `want` writes
/// must be in `given` with the same value. A wildcard fits only an equal or
/// wider wildcard: something that may be any text doesn't fit `text/plain`.
pub fn fits(want: &str, given: &str) -> bool {
    let (Some(w), Some(g)) = (parse(want), parse(given)) else { return false };
    let ty = w.ty == "*" || (w.ty == g.ty && (w.sub == "*" || w.sub == g.sub));
    ty && w.params.iter().all(|p| g.params.contains(p))
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
        let s = Stat { name: "a.txt".into(), dir: false, length: 12, ty: "text/plain".into(), version: 7 };
        let p = s.encode();
        assert_eq!(p.ty, STAT);
        assert_eq!(p.bytes.len(), 2 + 5 + 1 + 8 + 2 + 10 + 8);
        assert_eq!(Stat::decode(&p), Some(s));
        assert_eq!(Stat::decode(&Stat::dir("sub").encode()).unwrap().dir, true);
        assert_eq!(Stat::decode(&Payload::new(STAT, vec![1])), None);
        assert_eq!(Stat::decode(&Payload::text("hi")), None);
        let mut long = Stat::dir("x").encode();
        long.bytes.push(0);
        assert_eq!(Stat::decode(&long), None, "nothing may follow");
    }
}
