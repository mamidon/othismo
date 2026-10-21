//! Lexer, AST and parser for the image language (`.img` files).

use crate::ns::Verb;
use anyhow::{Result, bail};

/// A source position, 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for Pos {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub imports: Vec<Import>,
    pub requires: Vec<Require>,
    pub instances: Vec<Instance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub names: Vec<Ident>,
    pub path: String,
    pub pos: Pos,
}

/// `require <name> : <Type> [{ defaults }]`: a slot the host fills in.
#[derive(Debug, Clone, PartialEq)]
pub struct Require {
    pub name: Ident,
    /// The slot's type and any default fields, which need not be all of them.
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub name: Ident,
    /// A module name (`scripts/<name>.wasm`) or a dotted environment type,
    /// e.g. `Environment.Console`.
    pub type_name: Ident,
    pub mounts: Vec<Mount>,
    /// `send T { ... }`: environment config.
    pub config: Vec<Body>,
    /// `<verb> <path> [T { ... }]`: startup requests to this instance.
    pub requests: Vec<Request>,
    /// `include "<path>" [at <path>]`: files to pack into the image.
    pub includes: Vec<Include>,
}

/// `mount <instance> <path> [readonly]`
#[derive(Debug, Clone, PartialEq)]
pub struct Mount {
    pub target: Ident,
    pub at: Path,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub verb: Verb,
    pub verb_pos: Pos,
    pub path: Path,
    pub body: Option<Body>,
}

/// `include "<source>" [at <path>]`
#[derive(Debug, Clone, PartialEq)]
pub struct Include {
    /// A file or directory on the build machine, relative to the image.
    pub source: String,
    pub pos: Pos,
    pub at: Option<Path>,
}

/// A path literal, as written.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub path: String,
    pub pos: Pos,
}

/// A typed message literal: `T` or `T { field: value, ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub type_name: Ident,
    /// `None` for a unit message (`T` alone).
    pub fields: Option<Vec<(Ident, Value)>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i128),
    Float(f64),
    Bool(bool),
}

// ---------------------------------------------------------------------------
// Lexer

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    /// Starts with `/`: `/www`, `/hosts/localhost`.
    Path(String),
    Str(String),
    Int(i128),
    Float(f64),
    LBrace,
    RBrace,
    Comma,
    Colon,
    Eof,
}

impl std::fmt::Display for Tok {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Tok::Ident(s) => write!(f, "`{s}`"),
            Tok::Path(s) => write!(f, "path `{s}`"),
            Tok::Str(s) => write!(f, "string {s:?}"),
            Tok::Int(i) => write!(f, "integer {i}"),
            Tok::Float(x) => write!(f, "number {x}"),
            Tok::LBrace => write!(f, "`{{`"),
            Tok::RBrace => write!(f, "`}}`"),
            Tok::Comma => write!(f, "`,`"),
            Tok::Colon => write!(f, "`:`"),
            Tok::Eof => write!(f, "end of file"),
        }
    }
}

fn lex(src: &str) -> Result<Vec<(Tok, Pos)>> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let (mut i, mut line, mut col) = (0, 1, 1);

    macro_rules! bump {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        let pos = Pos { line, col };
        if c.is_whitespace() {
            bump!();
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                bump!();
            }
        } else if c == '/' {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '{' | '}' | ',') {
                bump!();
            }
            toks.push((Tok::Path(chars[start..i].iter().collect()), pos));
        } else if c == '{' || c == '}' || c == ',' || c == ':' {
            toks.push((
                match c {
                    '{' => Tok::LBrace,
                    '}' => Tok::RBrace,
                    ',' => Tok::Comma,
                    _ => Tok::Colon,
                },
                pos,
            ));
            bump!();
        } else if c == '"' {
            bump!();
            let mut s = String::new();
            loop {
                match chars.get(i) {
                    None | Some('\n') => bail!("{pos}: unterminated string literal"),
                    Some('"') => {
                        bump!();
                        break;
                    }
                    Some('\\') => {
                        bump!();
                        let esc = match chars.get(i) {
                            Some('n') => '\n',
                            Some('t') => '\t',
                            Some('r') => '\r',
                            Some('\\') => '\\',
                            Some('"') => '"',
                            other => bail!("{}: bad escape {:?}", Pos { line, col }, other),
                        };
                        s.push(esc);
                        bump!();
                    }
                    Some(&ch) => {
                        s.push(ch);
                        bump!();
                    }
                }
            }
            toks.push((Tok::Str(s), pos));
        } else if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            bump!();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                bump!();
            }
            let text: String = chars[start..i].iter().filter(|&&c| c != '_').collect();
            let tok = if text.contains(['.', 'e', 'E']) && !text.contains(['x', 'X']) {
                Tok::Float(text.parse().map_err(|_| anyhow::anyhow!("{pos}: bad number `{text}`"))?)
            } else {
                Tok::Int(text.parse().map_err(|_| anyhow::anyhow!("{pos}: bad integer `{text}`"))?)
            };
            toks.push((tok, pos));
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '$' | '.')) {
                bump!();
            }
            toks.push((Tok::Ident(chars[start..i].iter().collect()), pos));
        } else {
            bail!("{pos}: unexpected character {c:?}");
        }
    }
    toks.push((Tok::Eof, Pos { line, col }));
    Ok(toks)
}

// ---------------------------------------------------------------------------
// Parser

/// Words that can never be names. Member keywords (`mount`, `send`,
/// `include`, `readonly`, `at` and the verbs) only mean something inside an
/// instance block.
const KEYWORDS: &[&str] = &["import", "from", "require", "instance", "true", "false"];

/// Words that start a member of an instance block, besides the verbs.
const MEMBERS: &[&str] = &["mount", "send", "include"];

struct Parser {
    toks: Vec<(Tok, Pos)>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].0
    }

    fn pos(&self) -> Pos {
        self.toks[self.i].1
    }

    fn next(&mut self) -> (Tok, Pos) {
        let t = self.toks[self.i].clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == kw)
    }

    fn expect(&mut self, want: Tok) -> Result<Pos> {
        let (tok, pos) = self.next();
        if tok != want {
            bail!("{pos}: expected {want}, found {tok}");
        }
        Ok(pos)
    }

    fn expect_kw(&mut self, kw: &str) -> Result<Pos> {
        let (tok, pos) = self.next();
        match tok {
            Tok::Ident(s) if s == kw => Ok(pos),
            other => bail!("{pos}: expected `{kw}`, found {other}"),
        }
    }

    /// A plain identifier: not a keyword and (unless `dotted`) no dots.
    fn ident(&mut self, what: &str, dotted: bool) -> Result<Ident> {
        let (tok, pos) = self.next();
        match tok {
            Tok::Ident(name) if !KEYWORDS.contains(&name.as_str()) && (dotted || !name.contains('.')) => {
                Ok(Ident { name, pos })
            }
            other => bail!("{pos}: expected {what}, found {other}"),
        }
    }

    /// `a, b, c` — one or more identifiers separated by commas.
    fn ident_list(&mut self, what: &str) -> Result<Vec<Ident>> {
        let mut out = vec![self.ident(what, false)?];
        while *self.peek() == Tok::Comma {
            self.next();
            out.push(self.ident(what, false)?);
        }
        Ok(out)
    }

    fn path(&mut self) -> Result<Path> {
        match self.next() {
            (Tok::Path(path), pos) => Ok(Path { path, pos }),
            (other, pos) => bail!("{pos}: expected a path starting with `/`, found {other}"),
        }
    }

    fn image(&mut self) -> Result<Image> {
        let mut image = Image { imports: vec![], requires: vec![], instances: vec![] };
        loop {
            match self.peek().clone() {
                Tok::Eof => return Ok(image),
                Tok::Ident(kw) if kw == "import" => image.imports.push(self.import()?),
                Tok::Ident(kw) if kw == "instance" => image.instances.push(self.instance()?),
                Tok::Ident(kw) if kw == "require" => image.requires.push(self.require()?),
                Tok::Ident(kw) if kw == "protocol" => bail!(
                    "{}: protocols are gone; an instance's type names its module, and requests are addressed by path",
                    self.pos()
                ),
                other => bail!("{}: expected `import`, `require` or `instance`, found {other}", self.pos()),
            }
        }
    }

    fn import(&mut self) -> Result<Import> {
        let pos = self.expect_kw("import")?;
        self.expect(Tok::LBrace)?;
        let names = self.ident_list("type name")?;
        self.expect(Tok::RBrace)?;
        self.expect_kw("from")?;
        let path = match self.next() {
            (Tok::Str(s), _) => s,
            (other, pos) => bail!("{pos}: expected import path string, found {other}"),
        };
        Ok(Import { names, path, pos })
    }

    fn require(&mut self) -> Result<Require> {
        self.expect_kw("require")?;
        let name = self.ident("slot name", false)?;
        self.expect(Tok::Colon)?;
        let body = self.body()?;
        Ok(Require { name, body })
    }

    fn instance(&mut self) -> Result<Instance> {
        self.expect_kw("instance")?;
        let name = self.ident("instance name", false)?;
        self.expect(Tok::Colon)?;
        let type_name = self.ident("instance type", true)?;
        let mut inst = Instance { name, type_name, mounts: vec![], config: vec![], requests: vec![], includes: vec![] };
        if *self.peek() != Tok::LBrace {
            return Ok(inst);
        }
        self.next();
        loop {
            let (tok, pos) = self.next();
            let word = match tok {
                Tok::RBrace => return Ok(inst),
                Tok::Ident(word) => word,
                other => bail!("{pos}: expected `mount`, `send`, `include`, a verb or `}}`, found {other}"),
            };
            if word == "mount" {
                let target = self.ident("instance name", false)?;
                let at = self.path()?;
                let read_only = self.is_kw("readonly");
                if read_only {
                    self.next();
                }
                inst.mounts.push(Mount { target, at, read_only });
            } else if word == "send" {
                inst.config.push(self.body()?);
            } else if word == "include" {
                let source = match self.next() {
                    (Tok::Str(s), _) => s,
                    (other, pos) => bail!("{pos}: expected a quoted file or directory to include, found {other}"),
                };
                let at = if self.is_kw("at") {
                    self.next();
                    Some(self.path()?)
                } else {
                    None
                };
                inst.includes.push(Include { source, pos, at });
            } else if let Some(verb) = Verb::from_keyword(&word) {
                let path = self.path()?;
                let body = match self.peek() {
                    Tok::Ident(w) if !MEMBERS.contains(&w.as_str()) && Verb::from_keyword(w).is_none() => Some(self.body()?),
                    _ => None,
                };
                inst.requests.push(Request { verb, verb_pos: pos, path, body });
            } else {
                let verbs: Vec<&str> = crate::ns::VERBS.iter().map(|v| v.keyword()).collect();
                bail!("{pos}: expected `mount`, `send`, `include`, a verb ({}) or `}}`, found `{word}`", verbs.join(", "));
            }
        }
    }

    fn body(&mut self) -> Result<Body> {
        if *self.peek() == Tok::LBrace {
            bail!("{}: anonymous messages are not supported; write `TypeName {{ ... }}`", self.pos());
        }
        let type_name = self.ident("message type", false)?;
        if *self.peek() != Tok::LBrace {
            return Ok(Body { type_name, fields: None });
        }
        self.next();
        let mut fields = vec![];
        while *self.peek() != Tok::RBrace {
            let field = self.ident("field name", false)?;
            self.expect(Tok::Colon)?;
            let value = match self.next() {
                (Tok::Str(s), _) => Value::Str(s),
                (Tok::Int(i), _) => Value::Int(i),
                (Tok::Float(x), _) => Value::Float(x),
                (Tok::Ident(s), _) if s == "true" => Value::Bool(true),
                (Tok::Ident(s), _) if s == "false" => Value::Bool(false),
                (other, pos) => bail!("{pos}: expected a value, found {other}"),
            };
            fields.push((field, value));
            if *self.peek() == Tok::Comma {
                self.next();
            } else if *self.peek() != Tok::RBrace {
                bail!("{}: expected `,` or `}}`, found {}", self.pos(), self.peek());
            }
        }
        self.next();
        Ok(Body { type_name, fields: Some(fields) })
    }
}

pub fn parse(src: &str) -> Result<Image> {
    Parser { toks: lex(src)?, i: 0 }.image()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ids: &[Ident]) -> Vec<&str> {
        ids.iter().map(|i| i.name.as_str()).collect()
    }

    #[test]
    fn parses_full_syntax() {
        let img = parse(
            r#"
            import { Greeting, Ping } from "./messages"   // trailing comment
            import { Tick } from "./other"

            instance cons : Environment.Console
            instance net : Environment.Tcp { send Listen { address: "127.0.0.1:0" } }
            instance echo : Echo {
              mount cons /dev/cons
              mount net /net/tcp readonly
              write /greeting Greeting { prefix: "hi \"there\"", n: -3, x: 1.5, ok: true }
              invoke / Ping
              remove /old
              watch /
            }
            "#,
        )
        .unwrap();

        assert_eq!(img.imports.len(), 2);
        assert_eq!(names(&img.imports[0].names), ["Greeting", "Ping"]);
        assert_eq!(img.imports[0].path, "./messages");

        assert_eq!(img.instances.len(), 3);
        assert_eq!(img.instances[0].type_name.name, "Environment.Console");
        let net = &img.instances[1];
        assert_eq!(net.config[0].type_name.name, "Listen");
        assert_eq!(net.config[0].fields.as_ref().unwrap()[0].1, Value::Str("127.0.0.1:0".into()));

        let echo = &img.instances[2];
        let mounts: Vec<_> = echo.mounts.iter().map(|m| (m.target.name.as_str(), m.at.path.as_str(), m.read_only)).collect();
        assert_eq!(mounts, [("cons", "/dev/cons", false), ("net", "/net/tcp", true)]);

        let reqs = &echo.requests;
        assert_eq!(reqs.iter().map(|r| r.verb).collect::<Vec<_>>(), [Verb::Write, Verb::Invoke, Verb::Remove, Verb::Watch]);
        assert_eq!(reqs[0].path.path, "/greeting");
        let f = reqs[0].body.as_ref().unwrap().fields.as_ref().unwrap();
        assert_eq!(f[0].1, Value::Str("hi \"there\"".into()));
        assert_eq!(f[1].1, Value::Int(-3));
        assert_eq!(f[2].1, Value::Float(1.5));
        assert_eq!(f[3].1, Value::Bool(true));
        assert_eq!(reqs[1].path.path, "/");
        assert_eq!(reqs[1].body.as_ref().unwrap().fields, None);
        assert_eq!(reqs[2].body, None, "a verb ends a body-less request");
        assert_eq!(reqs[3].body, None);
    }

    #[test]
    fn parses_requires() {
        let img = parse(
            r#"require listen : Listen { address: "127.0.0.1:8080" }
            require admin : Text
            instance net : Environment.Tcp { send listen }"#,
        )
        .unwrap();
        let reqs: Vec<_> = img.requires.iter().map(|r| (r.name.name.as_str(), r.body.type_name.name.as_str())).collect();
        assert_eq!(reqs, [("listen", "Listen"), ("admin", "Text")]);
        assert_eq!(img.requires[0].body.fields.as_ref().unwrap()[0].1, Value::Str("127.0.0.1:8080".into()));
        assert_eq!(img.requires[1].body.fields, None);
        assert_eq!(img.instances[0].config[0].type_name.name, "listen", "a slot is used like a unit message");
        assert!(parse("require : Text").is_err());
        assert!(parse("instance require : P").is_err(), "`require` is a keyword");
    }

    #[test]
    fn parses_includes() {
        let img = parse(
            r#"instance files : Packed.Files {
              include "./public/"
              read /
              include "robots.txt" at /robots.txt
            }"#,
        )
        .unwrap();
        let inst = &img.instances[0];
        let incs: Vec<_> = inst.includes.iter().map(|i| (i.source.as_str(), i.at.as_ref().map(|p| p.path.as_str()))).collect();
        assert_eq!(incs, [("./public/", None), ("robots.txt", Some("/robots.txt"))]);
        assert_eq!(inst.requests[0].body, None, "`include` ends a body-less request");
        let e = parse("instance f : Packed.Files { include public/ }").unwrap_err().to_string();
        assert!(e.contains("expected a quoted file or directory"), "{e}");
    }

    #[test]
    fn comments_are_not_paths() {
        let img = parse("instance a : A { mount b / // the root\n mount c /c//d }").unwrap();
        let ats: Vec<&str> = img.instances[0].mounts.iter().map(|m| m.at.path.as_str()).collect();
        assert_eq!(ats, ["/", "/c//d"], "`//` only starts a comment at the start of a token");
    }

    #[test]
    fn rejects_malformed_members() {
        let e = parse("instance a : A { mount b }").unwrap_err().to_string();
        assert!(e.contains("expected a path"), "{e}");
        let e = parse("instance a : A { write hosts }").unwrap_err().to_string();
        assert!(e.contains("expected a path"), "{e}");
        let e = parse("instance a : A { frobnicate /x }").unwrap_err().to_string();
        assert!(e.contains("a verb"), "{e}");
        let e = parse("protocol P { in A }").unwrap_err().to_string();
        assert!(e.contains("protocols are gone"), "{e}");
    }

    #[test]
    fn rejects_anonymous_messages() {
        let err = parse("instance a : P { send { x: 1 } }").unwrap_err();
        assert!(err.to_string().contains("anonymous"), "{err}");
    }

    #[test]
    fn reports_position() {
        let err = parse("instance a : A {\n  mount b /b\n  bogus\n}").unwrap_err();
        assert!(err.to_string().starts_with("3:3:"), "{err}");
    }

    #[test]
    fn keywords_are_not_names() {
        assert!(parse("instance instance : P").is_err());
        assert!(parse("instance a.b : P").is_err());
        assert!(parse("instance write : P").is_ok(), "verbs are only keywords inside blocks");
    }
}
