//! Content types, as payloads carry them and headers write them.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;

/// What a payload's bytes are, such as `text/plain; charset=utf-8`.
///
/// Any text is a content type: the runtime carries it without looking, and
/// a server may answer with one nobody else has heard of. One that isn't
/// [well-formed](ContentType::is_well_formed) fits nowhere.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentType(Cow<'static, str>);

impl ContentType {
    /// Text, in UTF-8.
    pub const TEXT: ContentType = ContentType(Cow::Borrowed("text/plain"));
    /// Text of any kind.
    pub const ANY_TEXT: ContentType = ContentType(Cow::Borrowed("text/*"));
    /// Bytes with no more particular type.
    pub const OCTETS: ContentType = ContentType(Cow::Borrowed("application/octet-stream"));
    /// A stat record, which headers can write as [`STAT_NAME`](ContentType::STAT_NAME).
    pub const STAT: ContentType = ContentType(Cow::Borrowed("application/x-othismo-stat"));
    /// What headers write for a stat record's content type.
    pub const STAT_NAME: &str = "Stat";
    /// Any payload at all.
    pub const ANY: ContentType = ContentType(Cow::Borrowed("*/*"));

    /// A content type written into the program, without copying it.
    pub const fn from_static(text: &'static str) -> ContentType {
        ContentType(Cow::Borrowed(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether it is `type/subtype`, `type/*` or `*/*`, optionally followed
    /// by `; name=value` parameters.
    pub fn is_well_formed(&self) -> bool {
        self.parse().is_some()
    }

    /// Whether a payload of this content type fits where `want` is written.
    /// `*/*` takes anything and `text/*` any text; a parameter `want` writes
    /// must be here with the same value. A wildcard fits only an equal or
    /// wider wildcard: something that may be any text doesn't fit `text/plain`.
    pub fn fits(&self, want: &ContentType) -> bool {
        let (Some(want), Some(given)) = (want.parse(), self.parse()) else { return false };
        let types_fit = want.type_name == "*"
            || (want.type_name == given.type_name && (want.subtype == "*" || want.subtype == given.subtype));
        types_fit && want.parameters.iter().all(|parameter| given.parameters.contains(parameter))
    }

    /// Whether it is text of any kind, `text/*`.
    pub fn is_text(&self) -> bool {
        self.fits(&ContentType::ANY_TEXT)
    }

    fn parse(&self) -> Option<Parsed> {
        let mut parts = self.0.split(';');
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
        Some(Parsed { type_name: type_name.to_ascii_lowercase(), subtype: subtype.to_ascii_lowercase(), parameters })
    }
}

/// A well-formed content type taken apart: its type and subtype, lowercase,
/// and its parameters, with lowercase names.
struct Parsed {
    type_name: String,
    subtype: String,
    parameters: Vec<(String, String)>,
}

impl From<String> for ContentType {
    fn from(text: String) -> ContentType {
        ContentType(Cow::Owned(text))
    }
}

impl From<&str> for ContentType {
    fn from(text: &str) -> ContentType {
        ContentType(Cow::Owned(text.to_string()))
    }
}

impl From<ContentType> for String {
    fn from(content_type: ContentType) -> String {
        content_type.0.into_owned()
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fits(want: &str, given: &str) -> bool {
        ContentType::from(given).fits(&want.into())
    }

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
        assert!(!fits("*/*", "nonsense"), "what isn't well-formed fits nowhere");
    }

    #[test]
    fn any_text_is_a_content_type_but_only_some_are_well_formed() {
        for ok in ["text/plain", "application/x-othismo-stat", "text/*", "*/*", "text/html; charset=utf-8", "image/svg+xml"] {
            assert!(ContentType::from(ok).is_well_formed(), "{ok}");
        }
        for bad in ["text", "*/plain", "text/", "/plain", "text/plain;", "text/plain; charset", "te xt/plain"] {
            assert!(!ContentType::from(bad).is_well_formed(), "{bad}");
            assert_eq!(ContentType::from(bad).as_str(), bad, "kept as it is");
        }
    }
}
