//! Payloads: what a request or an answer carries besides its verb, path and
//! status. A payload is a content type and some bytes, which the runtime
//! carries without looking inside (see `docs/protocol.md`). The one layout
//! the protocol defines itself is the stat record, which `stat` and `list`
//! answer with.

use crate::content_type::ContentType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub content_type: ContentType,
    pub bytes: Vec<u8>,
}

impl Payload {
    pub fn new(content_type: impl Into<ContentType>, bytes: impl Into<Vec<u8>>) -> Payload {
        Payload { content_type: content_type.into(), bytes: bytes.into() }
    }

    /// `text/plain`.
    pub fn text(text: &str) -> Payload {
        Payload::new(ContentType::TEXT, text.as_bytes())
    }

    /// Whether it is text of any kind, `text/*`.
    pub fn is_text(&self) -> bool {
        self.content_type.is_text()
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
    /// The content type a `read` of the entry would answer with, if it is
    /// known. Not the record's own, which is always
    /// [`STAT`](ContentType::STAT): this is how a listing says what each
    /// entry holds without reading it.
    pub content_type: Option<ContentType>,
    /// Changes whenever what a `read` would answer changes, or 0.
    pub version: u64,
}

impl Stat {
    pub fn file(name: &str, length: u64, content_type: impl Into<ContentType>) -> Stat {
        Stat { name: name.into(), is_directory: false, length, content_type: Some(content_type.into()), version: 0 }
    }

    pub fn directory(name: &str) -> Stat {
        Stat { name: name.into(), is_directory: true, length: 0, content_type: None, version: 0 }
    }

    /// `name[s] kind[1] length[8] type[s] version[8]`, little-endian, where
    /// `[s]` is a string with a 2-byte length first, and an unknown type is
    /// empty.
    pub fn encode(&self) -> Payload {
        let mut bytes = vec![];
        Stat::put_string(&mut bytes, &self.name);
        bytes.push(self.is_directory as u8);
        bytes.extend_from_slice(&self.length.to_le_bytes());
        Stat::put_string(&mut bytes, self.content_type.as_ref().map_or("", ContentType::as_str));
        bytes.extend_from_slice(&self.version.to_le_bytes());
        Payload::new(ContentType::STAT, bytes)
    }

    fn put_string(bytes: &mut Vec<u8>, string: &str) {
        let length = u16::try_from(string.len()).expect("a stat record's strings are short");
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(string.as_bytes());
    }

    /// The stat record in `payload`, if it is a well-formed one.
    pub fn decode(payload: &Payload) -> Option<Stat> {
        if payload.content_type != ContentType::STAT {
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
        let content_type = Some(reader.take_string()?).filter(|text| !text.is_empty()).map(ContentType::from);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_records_round_trip() {
        let stat = Stat { name: "a.txt".into(), is_directory: false, length: 12, content_type: Some(ContentType::TEXT), version: 7 };
        let payload = stat.encode();
        assert_eq!(payload.content_type, ContentType::STAT);
        assert_eq!(payload.bytes.len(), 2 + 5 + 1 + 8 + 2 + 10 + 8);
        assert_eq!(Stat::decode(&payload), Some(stat));
        assert_eq!(Stat::decode(&Stat::directory("sub").encode()).unwrap().is_directory, true);
        assert_eq!(Stat::decode(&Payload::new(ContentType::STAT, vec![1])), None);
        assert_eq!(Stat::decode(&Payload::text("hi")), None);
        let mut long = Stat::directory("x").encode();
        long.bytes.push(0);
        assert_eq!(Stat::decode(&long), None, "nothing may follow");
    }
}
