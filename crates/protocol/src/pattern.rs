//! Path patterns, as headers write them: `/hits`, `/hosts/{name}`, `/**`.
//!
//! A literal segment matches itself, `{name}` matches exactly one non-empty
//! segment, and `**` (last only) matches the rest of the path, zero or more
//! segments. A trailing `/` is an empty last segment, so it counts, as it
//! does everywhere else. Where several patterns match, the most specific
//! wins: see [`Pattern::specificity`].

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Lit(String),
    Name,
    Rest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Pattern {
    text: String,
    segs: Vec<Seg>,
}

impl Pattern {
    pub fn parse(text: &str) -> Result<Pattern, String> {
        let Some(rest) = text.strip_prefix('/') else {
            return Err(format!("`{text}` is not a path pattern; it must start with `/`"));
        };
        let parts: Vec<&str> = rest.split('/').collect();
        let mut segs = vec![];
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let seg = match *part {
                "**" if last => Seg::Rest,
                "**" => return Err(format!("`{text}`: `**` can only come last")),
                "" if last => Seg::Lit(String::new()),
                "" => return Err(format!("`{text}` has an empty segment")),
                "." | ".." => return Err(format!("`{text}` has a `{part}` segment")),
                p if p.starts_with('{') && p.ends_with('}') => {
                    let name = &p[1..p.len() - 1];
                    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        return Err(format!("`{text}`: `{p}` should be `{{name}}`"));
                    }
                    Seg::Name
                }
                p if p.contains(['{', '}']) || p.contains("**") => {
                    return Err(format!("`{text}`: `{p}` mixes a literal with `{{name}}` or `**`"));
                }
                p => Seg::Lit(p.to_string()),
            };
            segs.push(seg);
        }
        Ok(Pattern { text: text.to_string(), segs })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether the pattern names one path only: no `{name}` and no `**`.
    pub fn is_literal(&self) -> bool {
        self.segs.iter().all(|s| matches!(s, Seg::Lit(_)))
    }

    /// Whether the clean path `path` matches.
    pub fn matches(&self, path: &str) -> bool {
        let path: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').collect();
        fn go(p: &[Seg], s: &[&str]) -> bool {
            match (p.first(), s.first()) {
                (Some(Seg::Rest), _) => true,
                (None, None) => true,
                (Some(Seg::Lit(l)), Some(seg)) => l == seg && go(&p[1..], &s[1..]),
                (Some(Seg::Name), Some(seg)) => !seg.is_empty() && go(&p[1..], &s[1..]),
                _ => false,
            }
        }
        go(&self.segs, &path)
    }

    /// Which of two patterns wins where both match: a literal over
    /// `{name}`, `{name}` over `**`, and longer over shorter, so `/www`
    /// wins over `/www/**`. `Equal` only for patterns of the same shape.
    pub fn specificity(&self, other: &Pattern) -> Ordering {
        // Running out where the other has `**` is the more specific of the two.
        fn rank(s: Option<&Seg>) -> u8 {
            match s {
                Some(Seg::Lit(_)) => 3,
                Some(Seg::Name) => 2,
                None => 1,
                Some(Seg::Rest) => 0,
            }
        }
        for i in 0..self.segs.len().max(other.segs.len()) {
            let (a, b) = (self.segs.get(i), other.segs.get(i));
            match rank(a).cmp(&rank(b)) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        Ordering::Equal
    }

    /// Whether every path `other` matches, this one matches too.
    pub fn covers(&self, other: &Pattern) -> bool {
        fn go(p: &[Seg], q: &[Seg]) -> bool {
            match (p.first(), q.first()) {
                (Some(Seg::Rest), _) => true,
                (None, None) => true,
                (Some(Seg::Lit(a)), Some(Seg::Lit(b))) => a == b && go(&p[1..], &q[1..]),
                (Some(Seg::Name), Some(Seg::Lit(b))) => !b.is_empty() && go(&p[1..], &q[1..]),
                (Some(Seg::Name), Some(Seg::Name)) => go(&p[1..], &q[1..]),
                _ => false,
            }
        }
        go(&self.segs, &other.segs)
    }

    /// What this pattern matches below the clean path `path`, relative to
    /// it: `/{name}` for `/hosts/{name}` below `/hosts`, `""` for the path
    /// itself, and `/**` wherever a `**` takes over. `None` if it matches
    /// nothing below `path`.
    pub fn below(&self, path: &str) -> Option<String> {
        let path: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').filter(|s| !s.is_empty()).collect();
        if path.is_empty() && self.text == "/" {
            return Some(String::new());
        }
        let mut segs = &self.segs[..];
        for seg in &path {
            match segs.first() {
                Some(Seg::Rest) => return Some("/**".into()),
                Some(Seg::Lit(l)) if l == seg => segs = &segs[1..],
                Some(Seg::Name) => segs = &segs[1..],
                _ => return None,
            }
        }
        let rest: Vec<String> = segs
            .iter()
            .map(|s| match s {
                Seg::Lit(l) => l.clone(),
                Seg::Name => "{x}".into(),
                Seg::Rest => "**".into(),
            })
            .collect();
        // Keep the names as written, which the text has and the segments don't.
        let written: Vec<&str> = self.text.strip_prefix('/').unwrap_or(&self.text).split('/').collect();
        let rest: Vec<String> = rest.iter().zip(&written[written.len() - rest.len()..]).map(|(_, w)| w.to_string()).collect();
        Some(if rest.is_empty() { String::new() } else { format!("/{}", rest.join("/")) })
    }

    /// `rel`, a pattern [`below`](Pattern::below) another path, put under
    /// the mount point `at` instead.
    pub fn under(at: &str, rel: &str) -> Pattern {
        let text = match (at, rel) {
            ("/", "") => "/".to_string(),
            ("/", rel) => rel.to_string(),
            (at, rel) => format!("{}{rel}", at.trim_end_matches('/')),
        };
        Pattern::parse(&text).expect("a pattern under a mount point is a pattern")
    }

    /// Whether some path matches both.
    pub fn overlaps(&self, other: &Pattern) -> bool {
        fn go(p: &[Seg], q: &[Seg]) -> bool {
            match (p.first(), q.first()) {
                (Some(Seg::Rest), _) | (_, Some(Seg::Rest)) => true,
                (None, None) => true,
                (Some(Seg::Lit(a)), Some(Seg::Lit(b))) => a == b && go(&p[1..], &q[1..]),
                (Some(Seg::Lit(a)), Some(Seg::Name)) | (Some(Seg::Name), Some(Seg::Lit(a))) => {
                    !a.is_empty() && go(&p[1..], &q[1..])
                }
                (Some(Seg::Name), Some(Seg::Name)) => go(&p[1..], &q[1..]),
                _ => false,
            }
        }
        go(&self.segs, &other.segs)
    }
}

/// The most specific of `items` whose pattern matches `path`.
pub fn best<'a, T>(items: &'a [T], pattern: impl Fn(&T) -> &Pattern, path: &str) -> Option<&'a T> {
    items.iter().filter(|i| pattern(i).matches(path)).max_by(|a, b| pattern(a).specificity(pattern(b)))
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl TryFrom<String> for Pattern {
    type Error = String;
    fn try_from(s: String) -> Result<Pattern, String> {
        Pattern::parse(&s)
    }
}

impl From<Pattern> for String {
    fn from(p: Pattern) -> String {
        p.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_move_below_one_path_and_under_another() {
        let below = |p: &str, path: &str| Pattern::parse(p).unwrap().below(path);
        assert_eq!(below("/hosts/{name}", "/hosts").as_deref(), Some("/{name}"));
        assert_eq!(below("/hosts", "/hosts").as_deref(), Some(""));
        assert_eq!(below("/hosts/", "/hosts").as_deref(), Some("/"));
        assert_eq!(below("/**", "/a/b").as_deref(), Some("/**"));
        assert_eq!(below("/{conn}/data", "/7").as_deref(), Some("/data"));
        assert_eq!(below("/x", "/"), Some("/x".into()));
        assert_eq!(below("/", "/").as_deref(), Some(""));
        assert_eq!(below("/hits", "/hosts"), None);
        assert_eq!(Pattern::under("/web", "/{name}").as_str(), "/web/{name}");
        assert_eq!(Pattern::under("/web", "").as_str(), "/web");
        assert_eq!(Pattern::under("/web/", "/").as_str(), "/web/");
        assert_eq!(Pattern::under("/", "").as_str(), "/");
        assert_eq!(Pattern::under("/", "/x").as_str(), "/x");
    }

    fn p(s: &str) -> Pattern {
        Pattern::parse(s).unwrap()
    }

    #[test]
    fn matches_literals_names_and_rest() {
        assert!(p("/hits").matches("/hits"));
        assert!(!p("/hits").matches("/hits/"), "a trailing `/` counts");
        assert!(p("/").matches("/"));
        assert!(!p("/").matches("/a"));
        assert!(p("/hosts/{name}").matches("/hosts/a"));
        assert!(!p("/hosts/{name}").matches("/hosts/"), "`{{name}}` is never empty");
        assert!(!p("/hosts/{name}").matches("/hosts/a/b"));
        assert!(p("/www/**").matches("/www"), "`**` matches zero segments");
        assert!(p("/www/**").matches("/www/"));
        assert!(p("/www/**").matches("/www/a/b/"));
        assert!(!p("/www/**").matches("/wwwx"));
        assert!(p("/**").matches("/"));
        assert!(p("/{a}/x/**").matches("/q/x"));
    }

    #[test]
    fn literals_name_one_path() {
        assert!(p("/net/tcp").is_literal());
        assert!(p("/").is_literal());
        assert!(!p("/{x}").is_literal());
        assert!(!p("/a/**").is_literal());
    }

    #[test]
    fn rejects_malformed_patterns() {
        for bad in ["hits", "/a/**/b", "/a//b", "/a/../b", "/{}", "/{a-b}", "/a{b}", "/a**"] {
            assert!(Pattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_most_specific_wins() {
        let order = |a: &str, b: &str| p(a).specificity(&p(b));
        assert_eq!(order("/hits", "/**"), Ordering::Greater);
        assert_eq!(order("/www", "/www/**"), Ordering::Greater);
        assert_eq!(order("/a/{x}", "/a/**"), Ordering::Greater);
        assert_eq!(order("/a/b/**", "/a/**"), Ordering::Greater);
        assert_eq!(order("/{x}", "/{y}"), Ordering::Equal);
        let items = [p("/**"), p("/hits"), p("/{x}")];
        let best = |path| best(&items, |i| i, path).map(Pattern::as_str);
        assert_eq!(best("/hits"), Some("/hits"));
        assert_eq!(best("/other"), Some("/{x}"));
        assert_eq!(best("/a/b"), Some("/**"));
    }

    #[test]
    fn covers_and_overlaps() {
        assert!(p("/**").covers(&p("/a/{x}")));
        assert!(p("/a/**").covers(&p("/a")));
        assert!(p("/{x}").covers(&p("/a")));
        assert!(!p("/{x}").covers(&p("/")), "`/` is an empty segment");
        assert!(!p("/a/{x}").covers(&p("/a/**")));
        assert!(!p("/a").covers(&p("/{x}")));
        assert!(p("/a/{x}").overlaps(&p("/a/**")));
        assert!(p("/{x}").overlaps(&p("/hits")));
        assert!(!p("/a").overlaps(&p("/b/**")));
        assert!(!p("/{x}").overlaps(&p("/")));
    }
}
