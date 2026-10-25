//! Path patterns, as headers write them: `/hits`, `/hosts/{name}`, `/**`.
//!
//! A literal segment matches itself, `{name}` matches exactly one non-empty
//! segment, and `**` (last only) matches the rest of the path, zero or more
//! segments. A trailing `/` is an empty last segment, so it counts, as it
//! does everywhere else. Where several patterns match, the most specific
//! wins: see [`Pattern::specificity`].

use crate::path::{MountPoint, PathName};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Name,
    Rest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Pattern {
    text: String,
    segments: Vec<Segment>,
}

impl Pattern {
    pub fn parse(text: &str) -> Result<Pattern, String> {
        let Some(rest) = text.strip_prefix('/') else {
            return Err(format!("`{text}` is not a path pattern; it must start with `/`"));
        };
        let parts: Vec<&str> = rest.split('/').collect();
        let mut segments = vec![];
        for (index, part) in parts.iter().enumerate() {
            let last = index + 1 == parts.len();
            let segment = match *part {
                "**" if last => Segment::Rest,
                "**" => return Err(format!("`{text}`: `**` can only come last")),
                "" if last => Segment::Literal(String::new()),
                "" => return Err(format!("`{text}` has an empty segment")),
                "." | ".." => return Err(format!("`{text}` has a `{part}` segment")),
                part if part.starts_with('{') && part.ends_with('}') => {
                    let name = &part[1..part.len() - 1];
                    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        return Err(format!("`{text}`: `{part}` should be `{{name}}`"));
                    }
                    Segment::Name
                }
                part if part.contains(['{', '}']) || part.contains("**") => {
                    return Err(format!("`{text}`: `{part}` mixes a literal with `{{name}}` or `**`"));
                }
                part => Segment::Literal(part.to_string()),
            };
            segments.push(segment);
        }
        Ok(Pattern { text: text.to_string(), segments })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether the pattern names one path only: no `{name}` and no `**`.
    pub fn is_literal(&self) -> bool {
        self.segments.iter().all(|segment| matches!(segment, Segment::Literal(_)))
    }

    pub fn matches(&self, path: &PathName) -> bool {
        let path: Vec<&str> = path.as_str()[1..].split('/').collect();
        fn matches(pattern: &[Segment], path: &[&str]) -> bool {
            match (pattern.first(), path.first()) {
                (Some(Segment::Rest), _) => true,
                (None, None) => true,
                (Some(Segment::Literal(literal)), Some(segment)) => literal == segment && matches(&pattern[1..], &path[1..]),
                (Some(Segment::Name), Some(segment)) => !segment.is_empty() && matches(&pattern[1..], &path[1..]),
                _ => false,
            }
        }
        matches(&self.segments, &path)
    }

    /// Which of two patterns wins where both match: a literal over
    /// `{name}`, `{name}` over `**`, and longer over shorter, so `/www`
    /// wins over `/www/**`. `Equal` only for patterns of the same shape.
    pub fn specificity(&self, other: &Pattern) -> Ordering {
        // Running out where the other has `**` is the more specific of the two.
        fn rank(segment: Option<&Segment>) -> u8 {
            match segment {
                Some(Segment::Literal(_)) => 3,
                Some(Segment::Name) => 2,
                None => 1,
                Some(Segment::Rest) => 0,
            }
        }
        for index in 0..self.segments.len().max(other.segments.len()) {
            match rank(self.segments.get(index)).cmp(&rank(other.segments.get(index))) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
        }
        Ordering::Equal
    }

    /// Whether every path `other` matches, this one matches too.
    pub fn covers(&self, other: &Pattern) -> bool {
        fn covers(pattern: &[Segment], other: &[Segment]) -> bool {
            match (pattern.first(), other.first()) {
                (Some(Segment::Rest), _) => true,
                (None, None) => true,
                (Some(Segment::Literal(a)), Some(Segment::Literal(b))) => a == b && covers(&pattern[1..], &other[1..]),
                (Some(Segment::Name), Some(Segment::Literal(b))) => !b.is_empty() && covers(&pattern[1..], &other[1..]),
                (Some(Segment::Name), Some(Segment::Name)) => covers(&pattern[1..], &other[1..]),
                _ => false,
            }
        }
        covers(&self.segments, &other.segments)
    }

    /// What this pattern matches below `path`, relative to
    /// it: `/{name}` for `/hosts/{name}` below `/hosts`, `""` for the path
    /// itself, and `/**` wherever a `**` takes over. `None` if it matches
    /// nothing below `path`.
    pub fn below(&self, path: &PathName) -> Option<String> {
        let path: Vec<&str> = path.as_str()[1..].split('/').filter(|segment| !segment.is_empty()).collect();
        if path.is_empty() && self.text == "/" {
            return Some(String::new());
        }
        let mut segments = &self.segments[..];
        for segment in &path {
            match segments.first() {
                Some(Segment::Rest) => return Some("/**".into()),
                Some(Segment::Literal(literal)) if literal == segment => segments = &segments[1..],
                Some(Segment::Name) => segments = &segments[1..],
                _ => return None,
            }
        }
        // The rest as written, which keeps the names that the segments don't.
        let written: Vec<&str> = self.text.strip_prefix('/').unwrap_or(&self.text).split('/').collect();
        let rest = &written[written.len() - segments.len()..];
        Some(if rest.is_empty() { String::new() } else { format!("/{}", rest.join("/")) })
    }

    /// `relative`, a pattern [`below`](Pattern::below) another path, put
    /// under the mount point `at` instead.
    pub fn under(at: &MountPoint, relative: &str) -> Pattern {
        let text = match (at.as_str(), relative) {
            ("/", "") => "/".to_string(),
            ("/", relative) => relative.to_string(),
            (at, relative) => format!("{at}{relative}"),
        };
        Pattern::parse(&text).expect("a pattern under a mount point is a pattern")
    }

    /// Whether some path matches both.
    pub fn overlaps(&self, other: &Pattern) -> bool {
        fn overlaps(pattern: &[Segment], other: &[Segment]) -> bool {
            match (pattern.first(), other.first()) {
                (Some(Segment::Rest), _) | (_, Some(Segment::Rest)) => true,
                (None, None) => true,
                (Some(Segment::Literal(a)), Some(Segment::Literal(b))) => a == b && overlaps(&pattern[1..], &other[1..]),
                (Some(Segment::Literal(literal)), Some(Segment::Name)) | (Some(Segment::Name), Some(Segment::Literal(literal))) => {
                    !literal.is_empty() && overlaps(&pattern[1..], &other[1..])
                }
                (Some(Segment::Name), Some(Segment::Name)) => overlaps(&pattern[1..], &other[1..]),
                _ => false,
            }
        }
        overlaps(&self.segments, &other.segments)
    }
}

/// The most specific of `items` whose pattern matches `path`.
pub fn best<'a, T>(items: &'a [T], pattern: impl Fn(&T) -> &Pattern, path: &PathName) -> Option<&'a T> {
    items.iter().filter(|item| pattern(item).matches(path)).max_by(|a, b| pattern(a).specificity(pattern(b)))
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl TryFrom<String> for Pattern {
    type Error = String;
    fn try_from(text: String) -> Result<Pattern, String> {
        Pattern::parse(&text)
    }
}

impl From<Pattern> for String {
    fn from(pattern: Pattern) -> String {
        pattern.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> PathName {
        PathName::parse(text).unwrap()
    }

    fn at(text: &str) -> MountPoint {
        MountPoint::parse(text).unwrap()
    }

    #[test]
    fn patterns_move_below_one_path_and_under_another() {
        let below = |pattern: &str, path: &str| Pattern::parse(pattern).unwrap().below(&self::path(path));
        assert_eq!(below("/hosts/{name}", "/hosts").as_deref(), Some("/{name}"));
        assert_eq!(below("/hosts", "/hosts").as_deref(), Some(""));
        assert_eq!(below("/hosts/", "/hosts").as_deref(), Some("/"));
        assert_eq!(below("/**", "/a/b").as_deref(), Some("/**"));
        assert_eq!(below("/{conn}/data", "/7").as_deref(), Some("/data"));
        assert_eq!(below("/x", "/"), Some("/x".into()));
        assert_eq!(below("/", "/").as_deref(), Some(""));
        assert_eq!(below("/hits", "/hosts"), None);
        assert_eq!(Pattern::under(&at("/web"), "/{name}").as_str(), "/web/{name}");
        assert_eq!(Pattern::under(&at("/web"), "").as_str(), "/web");
        assert_eq!(Pattern::under(&at("/web/"), "/").as_str(), "/web/");
        assert_eq!(Pattern::under(&at("/"), "").as_str(), "/");
        assert_eq!(Pattern::under(&at("/"), "/x").as_str(), "/x");
    }

    fn pattern(text: &str) -> Pattern {
        Pattern::parse(text).unwrap()
    }

    #[test]
    fn matches_literals_names_and_rest() {
        assert!(pattern("/hits").matches(&path("/hits")));
        assert!(!pattern("/hits").matches(&path("/hits/")), "a trailing `/` counts");
        assert!(pattern("/").matches(&path("/")));
        assert!(!pattern("/").matches(&path("/a")));
        assert!(pattern("/hosts/{name}").matches(&path("/hosts/a")));
        assert!(!pattern("/hosts/{name}").matches(&path("/hosts/")), "`{{name}}` is never empty");
        assert!(!pattern("/hosts/{name}").matches(&path("/hosts/a/b")));
        assert!(pattern("/www/**").matches(&path("/www")), "`**` matches zero segments");
        assert!(pattern("/www/**").matches(&path("/www/")));
        assert!(pattern("/www/**").matches(&path("/www/a/b/")));
        assert!(!pattern("/www/**").matches(&path("/wwwx")));
        assert!(pattern("/**").matches(&path("/")));
        assert!(pattern("/{a}/x/**").matches(&path("/q/x")));
    }

    #[test]
    fn literals_name_one_path() {
        assert!(pattern("/net/tcp").is_literal());
        assert!(pattern("/").is_literal());
        assert!(!pattern("/{x}").is_literal());
        assert!(!pattern("/a/**").is_literal());
    }

    #[test]
    fn rejects_malformed_patterns() {
        for bad in ["hits", "/a/**/b", "/a//b", "/a/../b", "/{}", "/{a-b}", "/a{b}", "/a**"] {
            assert!(Pattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_most_specific_wins() {
        let order = |a: &str, b: &str| pattern(a).specificity(&pattern(b));
        assert_eq!(order("/hits", "/**"), Ordering::Greater);
        assert_eq!(order("/www", "/www/**"), Ordering::Greater);
        assert_eq!(order("/a/{x}", "/a/**"), Ordering::Greater);
        assert_eq!(order("/a/b/**", "/a/**"), Ordering::Greater);
        assert_eq!(order("/{x}", "/{y}"), Ordering::Equal);
        let items = [pattern("/**"), pattern("/hits"), pattern("/{x}")];
        let best = |path| best(&items, |item| item, &self::path(path)).map(Pattern::as_str);
        assert_eq!(best("/hits"), Some("/hits"));
        assert_eq!(best("/other"), Some("/{x}"));
        assert_eq!(best("/a/b"), Some("/**"));
    }

    #[test]
    fn covers_and_overlaps() {
        assert!(pattern("/**").covers(&pattern("/a/{x}")));
        assert!(pattern("/a/**").covers(&pattern("/a")));
        assert!(pattern("/{x}").covers(&pattern("/a")));
        assert!(!pattern("/{x}").covers(&pattern("/")), "`/` is an empty segment");
        assert!(!pattern("/a/{x}").covers(&pattern("/a/**")));
        assert!(!pattern("/a").covers(&pattern("/{x}")));
        assert!(pattern("/a/{x}").overlaps(&pattern("/a/**")));
        assert!(pattern("/{x}").overlaps(&pattern("/hits")));
        assert!(!pattern("/a").overlaps(&pattern("/b/**")));
        assert!(!pattern("/{x}").overlaps(&pattern("/")));
    }
}
