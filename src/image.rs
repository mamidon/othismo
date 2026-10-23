//! Lexer, AST and parser for the image language (`.img` files). Headers
//! (`.iface` files, see [`crate::iface`]) are written in the same language
//! and share the lexer and parser.

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
    /// `import "<image>"`: other images, whose instances, mounts, slots and
    /// requests become this one's.
    pub images: Vec<Path>,
    pub requires: Vec<Require>,
    pub instances: Vec<Instance>,
    /// Top-level mounts: the image's own view, which its requests resolve in.
    pub mounts: Vec<Mount>,
    /// `<verb> <path> [T { ... }]`: startup requests, in the order they are made.
    pub requests: Vec<Request>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub names: Vec<Ident>,
    pub from: Source,
    pub pos: Pos,
}

/// Where an import's types come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// `from env` or `from Greeter`: the runtime's header, or a module's.
    Module(Ident),
    /// `from "./shared.iface"`: a file, relative to the importing one.
    Path(String),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Source::Module(m) => f.write_str(&m.name),
            Source::Path(p) => write!(f, "{p:?}"),
        }
    }
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
    /// The directory of the image that declared it, relative to this one's:
    /// `""` for its own, and an imported image's for one it imported.
    pub dir: String,
    /// Declared by an imported image, so the runtime's objects among them
    /// run in memory.
    pub imported: bool,
    /// A module name (`scripts/<name>.wasm`) or a dotted environment type,
    /// e.g. `Console`.
    pub type_name: Ident,
    pub mounts: Vec<Mount>,
    /// `include "<path>" [at <path>]`: files to pack into the image.
    pub includes: Vec<Include>,
}

/// `mount <path> <instance> [readonly]`
#[derive(Debug, Clone, PartialEq)]
pub struct Mount {
    pub at: Path,
    pub target: Ident,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub verb: Verb,
    pub verb_pos: Pos,
    /// A path in the image's view.
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
pub(crate) enum Tok {
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
    /// `->`, before what a request is answered with.
    Arrow,
    /// `*` after an answer's type: a stream.
    Star,
    /// A content type: `text/plain`, `text/*`, `*/*`, `text/html;charset=utf-8`.
    Type(String),
    /// A `##` comment, which documents what follows. Only headers keep them.
    Doc(String),
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
            Tok::Arrow => write!(f, "`->`"),
            Tok::Star => write!(f, "`*`"),
            Tok::Type(s) => write!(f, "content type `{s}`"),
            Tok::Doc(_) => write!(f, "a `##` comment"),
            Tok::Eof => write!(f, "end of file"),
        }
    }
}

/// Tokenizes `src`. With `docs`, `##` comments are kept as [`Tok::Doc`];
/// otherwise every comment is dropped.
pub(crate) fn lex(src: &str, docs: bool) -> Result<Vec<(Tok, Pos)>> {
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
        } else if c == '#' {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                bump!();
            }
            let text: String = chars[start..i].iter().collect();
            if docs && let Some(doc) = text.strip_prefix("##") {
                toks.push((Tok::Doc(doc.strip_prefix(' ').unwrap_or(doc).trim_end().to_string()), pos));
            }
        } else if c == '/' {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '}' | ',') {
                // `{` only belongs to a path as a `{name}` segment.
                if chars[i] == '{' {
                    if chars[i - 1] != '/' {
                        break;
                    }
                    while i < chars.len() && chars[i] != '}' && !chars[i].is_whitespace() {
                        bump!();
                    }
                    if chars.get(i) != Some(&'}') {
                        bail!("{pos}: unterminated `{{` in a path");
                    }
                }
                bump!();
            }
            toks.push((Tok::Path(chars[start..i].iter().collect()), pos));
        } else if c == '-' && chars.get(i + 1) == Some(&'>') {
            toks.push((Tok::Arrow, pos));
            bump!();
            bump!();
        } else if c == '*' && chars.get(i + 1) == Some(&'/') {
            let start = i;
            while i < chars.len() && type_char(chars[i]) {
                bump!();
            }
            push_type(&mut toks, chars[start..i].iter().collect(), pos, Pos { line, col });
        } else if c == '*' {
            toks.push((Tok::Star, pos));
            bump!();
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
            // An identifier straight before a `/` starts a content type.
            if chars.get(i) == Some(&'/') {
                while i < chars.len() && type_char(chars[i]) {
                    bump!();
                }
                push_type(&mut toks, chars[start..i].iter().collect(), pos, Pos { line, col });
                continue;
            }
            toks.push((Tok::Ident(chars[start..i].iter().collect()), pos));
        } else {
            bail!("{pos}: unexpected character {c:?}");
        }
    }
    toks.push((Tok::Eof, Pos { line, col }));
    Ok(toks)
}

/// Whether `c` can be part of a content type as headers write it.
fn type_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$&^_.+-*/;=".contains(c)
}

/// A content type, and a `*` straight after it, which marks a stream:
/// `text/plain*`, `text/**`, `*/**`. `text/*` alone is any text.
fn push_type(toks: &mut Vec<(Tok, Pos)>, mut ty: String, pos: Pos, end: Pos) {
    let stream = ty.ends_with("**") || (ty.ends_with('*') && !ty.ends_with("/*"));
    if stream {
        ty.pop();
    }
    toks.push((Tok::Type(ty), pos));
    if stream {
        toks.push((Tok::Star, Pos { line: end.line, col: end.col - 1 }));
    }
}

// ---------------------------------------------------------------------------
// Parser

/// Words that can never be names. Member keywords (`mount`, `include`,
/// `readonly` and `at`) only mean something inside an instance block, and
/// the verbs only where a request can start.
pub(crate) const KEYWORDS: &[&str] = &["import", "from", "require", "instance", "true", "false"];

/// Words that start a declaration at the top of an image, besides the verbs.
const DECLS: &[&str] = &["import", "require", "instance", "mount"];

pub(crate) struct Parser {
    toks: Vec<(Tok, Pos)>,
    i: usize,
}

impl Parser {
    pub(crate) fn new(src: &str, docs: bool) -> Result<Parser> {
        Ok(Parser { toks: lex(src, docs)?, i: 0 })
    }

    pub(crate) fn peek(&self) -> &Tok {
        &self.toks[self.i].0
    }

    pub(crate) fn pos(&self) -> Pos {
        self.toks[self.i].1
    }

    pub(crate) fn next(&mut self) -> (Tok, Pos) {
        let t = self.toks[self.i].clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }

    pub(crate) fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == kw)
    }

    pub(crate) fn expect(&mut self, want: Tok) -> Result<Pos> {
        let (tok, pos) = self.next();
        if tok != want {
            bail!("{pos}: expected {want}, found {tok}");
        }
        Ok(pos)
    }

    pub(crate) fn expect_kw(&mut self, kw: &str) -> Result<Pos> {
        let (tok, pos) = self.next();
        match tok {
            Tok::Ident(s) if s == kw => Ok(pos),
            other => bail!("{pos}: expected `{kw}`, found {other}"),
        }
    }

    /// A plain identifier: not a keyword and (unless `dotted`) no dots.
    pub(crate) fn ident(&mut self, what: &str, dotted: bool) -> Result<Ident> {
        let (tok, pos) = self.next();
        match tok {
            Tok::Ident(name) if !KEYWORDS.contains(&name.as_str()) && (dotted || !name.contains('.')) => {
                Ok(Ident { name, pos })
            }
            other => bail!("{pos}: expected {what}, found {other}"),
        }
    }

    /// `a, b, c` — one or more identifiers separated by commas.
    pub(crate) fn ident_list(&mut self, what: &str) -> Result<Vec<Ident>> {
        let mut out = vec![self.ident(what, false)?];
        while *self.peek() == Tok::Comma {
            self.next();
            out.push(self.ident(what, false)?);
        }
        Ok(out)
    }

    pub(crate) fn path(&mut self) -> Result<Path> {
        match self.next() {
            (Tok::Path(path), pos) => Ok(Path { path, pos }),
            (other, pos) => bail!("{pos}: expected a path starting with `/`, found {other}"),
        }
    }

    fn image(&mut self) -> Result<Image> {
        let mut image = Image { imports: vec![], images: vec![], requires: vec![], instances: vec![], mounts: vec![], requests: vec![] };
        loop {
            match self.peek().clone() {
                Tok::Eof => return Ok(image),
                Tok::Ident(kw) if kw == "import" && matches!(self.toks.get(self.i + 1), Some((Tok::Str(_), _))) => {
                    self.next();
                    let (Tok::Str(path), pos) = self.next() else { unreachable!("just matched") };
                    image.images.push(Path { path, pos });
                }
                Tok::Ident(kw) if kw == "import" => image.imports.push(self.import()?),
                Tok::Ident(kw) if kw == "instance" => image.instances.push(self.instance()?),
                Tok::Ident(kw) if kw == "require" => image.requires.push(self.require()?),
                Tok::Ident(kw) if kw == "mount" => {
                    self.next();
                    image.mounts.push(self.mount()?);
                }
                Tok::Ident(kw) if Verb::from_keyword(&kw).is_some() => image.requests.push(self.startup_request()?),
                other => {
                    bail!("{}: expected `import`, `require`, `instance`, `mount` or a verb, found {other}", self.pos())
                }
            }
        }
    }

    /// `<verb> <path> [T { ... }]`. A body-less request ends where the next
    /// declaration or request starts.
    fn startup_request(&mut self) -> Result<Request> {
        let (tok, verb_pos) = self.next();
        let verb = match tok {
            Tok::Ident(w) => Verb::from_keyword(&w),
            _ => None,
        }
        .ok_or_else(|| anyhow::anyhow!("{verb_pos}: expected a verb"))?;
        let path = self.path()?;
        let body = match self.peek() {
            Tok::Ident(w) if !DECLS.contains(&w.as_str()) && Verb::from_keyword(w).is_none() => Some(self.body()?),
            Tok::LBrace => Some(self.body()?),
            _ => None,
        };
        Ok(Request { verb, verb_pos, path, body })
    }

    /// `import { A, B } from env`, `from Module` or `from "./file.iface"`.
    pub(crate) fn import(&mut self) -> Result<Import> {
        let pos = self.expect_kw("import")?;
        self.expect(Tok::LBrace)?;
        let names = self.ident_list("type name")?;
        self.expect(Tok::RBrace)?;
        self.expect_kw("from")?;
        let from = match self.next() {
            (Tok::Str(s), _) => Source::Path(s),
            (Tok::Ident(name), pos) if !KEYWORDS.contains(&name.as_str()) => Source::Module(Ident { name, pos }),
            (other, pos) => bail!("{pos}: expected a module name or a quoted path to import from, found {other}"),
        };
        Ok(Import { names, from, pos })
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
        let mut inst = Instance { name, dir: String::new(), imported: false, type_name, mounts: vec![], includes: vec![] };
        if *self.peek() != Tok::LBrace {
            return Ok(inst);
        }
        self.next();
        loop {
            let (tok, pos) = self.next();
            let word = match tok {
                Tok::RBrace => return Ok(inst),
                Tok::Ident(word) => word,
                other => bail!("{pos}: expected `mount`, `include` or `}}`, found {other}"),
            };
            if word == "mount" {
                inst.mounts.push(self.mount()?);
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
            } else if Verb::from_keyword(&word).is_some() {
                let at = match self.peek() {
                    Tok::Path(p) if p != "/" => p.clone(),
                    _ => String::new(),
                };
                bail!(
                    "{pos}: requests are made at the top level, in the image's view; mount `{}` there, e.g. \
                     `mount /{0} {0}`, and then `{word} /{0}{at} ...`",
                    inst.name.name
                );
            } else {
                bail!("{pos}: expected `mount`, `include` or `}}`, found `{word}`");
            }
        }
    }

    /// What follows `mount`: `<path> <instance> [readonly]`.
    fn mount(&mut self) -> Result<Mount> {
        if let (Tok::Ident(target), Some((Tok::Path(at), _))) = (self.peek(), self.toks.get(self.i + 1)) {
            bail!("{}: the mount point comes first: `mount {at} {target}`", self.pos());
        }
        let at = self.path()?;
        let target = self.ident("instance name", false)?;
        let read_only = self.is_kw("readonly");
        if read_only {
            self.next();
        }
        Ok(Mount { at, target, read_only })
    }

    fn body(&mut self) -> Result<Body> {
        if *self.peek() == Tok::LBrace {
            bail!("{}: anonymous messages are not supported; write `TypeName {{ ... }}`", self.pos());
        }
        let type_name = self.ident("message type", false)?;
        if *self.peek() != Tok::LBrace {
            return Ok(Body { type_name, fields: None });
        }
        Ok(Body { type_name, fields: Some(self.fields()?) })
    }

    /// `{ field: value, ... }`
    fn fields(&mut self) -> Result<Vec<(Ident, Value)>> {
        self.expect(Tok::LBrace)?;
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
        Ok(fields)
    }

    /// A value file: `slot { field: value, ... }`, any number of times.
    fn values(&mut self) -> Result<Vec<SlotValues>> {
        let mut out = vec![];
        while *self.peek() != Tok::Eof {
            let slot = self.ident("slot name", false)?;
            let fields = self.fields()?;
            out.push(SlotValues { slot, fields });
        }
        Ok(out)
    }
}

pub fn parse(src: &str) -> Result<Image> {
    Parser::new(src, false)?.image()
}

/// One block of a value file.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotValues {
    pub slot: Ident,
    pub fields: Vec<(Ident, Value)>,
}

/// Parses a value file, which gives slots their values in the image's own
/// block syntax: `listen { address: "0.0.0.0:80" }`.
pub fn parse_values(src: &str) -> Result<Vec<SlotValues>> {
    Parser::new(src, false)?.values()
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
            import { Greeting, Ping } from "./messages"   # trailing comment
            import { Tick } from "./other"

            instance cons : Console
            instance net : Listener
            control /net Listen { address: "127.0.0.1:0" }
            instance echo : Echo {
              mount /dev/cons cons
              mount /net/tcp net readonly
            }
            write /echo/greeting Greeting { prefix: "hi \"there\"", n: -3, x: 1.5, ok: true }
            invoke /echo Ping
            remove /echo/old
            watch /echo
            "#,
        )
        .unwrap();

        assert_eq!(img.imports.len(), 2);
        assert_eq!(names(&img.imports[0].names), ["Greeting", "Ping"]);
        assert_eq!(img.imports[0].from, Source::Path("./messages".into()));

        assert_eq!(img.instances.len(), 3);
        assert_eq!(img.instances[0].type_name.name, "Console");
        let echo = &img.instances[2];
        let mounts: Vec<_> = echo.mounts.iter().map(|m| (m.target.name.as_str(), m.at.path.as_str(), m.read_only)).collect();
        assert_eq!(mounts, [("cons", "/dev/cons", false), ("net", "/net/tcp", true)]);

        let reqs = &img.requests;
        let verbs: Vec<_> = reqs.iter().map(|r| r.verb).collect();
        assert_eq!(verbs, [Verb::Control, Verb::Write, Verb::Invoke, Verb::Remove, Verb::Watch]);
        assert_eq!(reqs[0].path.path, "/net");
        assert_eq!(reqs[0].body.as_ref().unwrap().type_name.name, "Listen");
        assert_eq!(reqs[1].path.path, "/echo/greeting");
        let f = reqs[1].body.as_ref().unwrap().fields.as_ref().unwrap();
        assert_eq!(f[0].1, Value::Str("hi \"there\"".into()));
        assert_eq!(f[1].1, Value::Int(-3));
        assert_eq!(f[2].1, Value::Float(1.5));
        assert_eq!(f[3].1, Value::Bool(true));
        assert_eq!(reqs[2].path.path, "/echo");
        assert_eq!(reqs[2].body.as_ref().unwrap().fields, None);
        assert_eq!(reqs[3].body, None, "a verb ends a body-less request");
        assert_eq!(reqs[4].body, None);
    }

    #[test]
    fn requests_can_come_anywhere() {
        let img = parse("remove /a/x\ninstance a : A\nread /a\nrequire t : Text\nwatch /a").unwrap();
        let paths: Vec<_> = img.requests.iter().map(|r| (r.path.path.as_str(), r.body.is_some())).collect();
        assert_eq!(paths, [("/a/x", false), ("/a", false), ("/a", false)], "a declaration ends a body-less request");
    }

    #[test]
    fn parses_requires() {
        let img = parse(
            r#"require listen : Listen { address: "127.0.0.1:8080" }
            require admin : Text
            instance net : Listener
            control /net listen"#,
        )
        .unwrap();
        let reqs: Vec<_> = img.requires.iter().map(|r| (r.name.name.as_str(), r.body.type_name.name.as_str())).collect();
        assert_eq!(reqs, [("listen", "Listen"), ("admin", "Text")]);
        assert_eq!(img.requires[0].body.fields.as_ref().unwrap()[0].1, Value::Str("127.0.0.1:8080".into()));
        assert_eq!(img.requires[1].body.fields, None);
        let body = img.requests[0].body.as_ref().unwrap();
        assert_eq!(body.type_name.name, "listen", "a slot is used like a unit message");
        assert!(parse("require : Text").is_err());
        assert!(parse("instance require : P").is_err(), "`require` is a keyword");
    }

    #[test]
    fn parses_value_files() {
        let v = parse_values("# prod\nlisten { address: \"0.0.0.0:80\" }\ndb { port: 5432, tls: true }").unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].slot.name.as_str(), v[0].fields[0].0.pos), ("listen", Pos { line: 2, col: 10 }));
        assert_eq!(v[1].fields[1].1, Value::Bool(true));
        assert!(parse_values("listen address: 1").is_err());
        assert!(parse_values("listen : Listen { address: \"x\" }").is_err(), "the image knows the type");
    }

    #[test]
    fn parses_includes() {
        let img = parse(
            r#"instance files : Packed.Files {
              include "./public/"
              include "robots.txt" at /robots.txt
            }"#,
        )
        .unwrap();
        let inst = &img.instances[0];
        let incs: Vec<_> = inst.includes.iter().map(|i| (i.source.as_str(), i.at.as_ref().map(|p| p.path.as_str()))).collect();
        assert_eq!(incs, [("./public/", None), ("robots.txt", Some("/robots.txt"))]);
        let e = parse("instance f : Packed.Files { include public/ }").unwrap_err().to_string();
        assert!(e.contains("expected a quoted file or directory"), "{e}");
    }

    #[test]
    fn imports_name_a_module_or_a_path() {
        let img = parse("import { Line } from env\nimport { Greeting } from Greeter").unwrap();
        let froms: Vec<String> = img.imports.iter().map(|i| i.from.to_string()).collect();
        assert_eq!(froms, ["env", "Greeter"]);
        assert!(parse("import { Line } from instance").is_err(), "a keyword isn't a module");
    }

    #[test]
    fn paths_may_hold_name_segments() {
        let toks: Vec<Tok> = lex("/hosts/{name} /a/{b}/** /x{ /c}", false).unwrap().into_iter().map(|t| t.0).collect();
        assert_eq!(
            toks,
            [
                Tok::Path("/hosts/{name}".into()),
                Tok::Path("/a/{b}/**".into()),
                Tok::Path("/x".into()),
                Tok::LBrace,
                Tok::Path("/c".into()),
                Tok::RBrace,
                Tok::Eof
            ]
        );
        assert!(lex("/a/{b", false).is_err());
    }

    #[test]
    fn content_types_are_one_token_and_a_star_after_one_is_a_stream() {
        let toks: Vec<Tok> =
            lex("text/plain text/plain* text/* text/** */* */** application/x-othismo-stat text/html;charset=utf-8 a /b", false)
                .unwrap()
                .into_iter()
                .map(|t| t.0)
                .collect();
        let ty = |s: &str| Tok::Type(s.into());
        assert_eq!(
            toks,
            [
                ty("text/plain"),
                ty("text/plain"),
                Tok::Star,
                ty("text/*"),
                ty("text/*"),
                Tok::Star,
                ty("*/*"),
                ty("*/*"),
                Tok::Star,
                ty("application/x-othismo-stat"),
                ty("text/html;charset=utf-8"),
                Tok::Ident("a".into()),
                Tok::Path("/b".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn doc_comments_are_kept_only_when_asked() {
        let src = "## Says hi.\n# not a doc\nx -> Y*";
        let toks: Vec<Tok> = lex(src, true).unwrap().into_iter().map(|t| t.0).collect();
        assert_eq!(toks, [Tok::Doc("Says hi.".into()), Tok::Ident("x".into()), Tok::Arrow, Tok::Ident("Y".into()), Tok::Star, Tok::Eof]);
        assert_eq!(lex(src, false).unwrap()[0].0, Tok::Ident("x".into()));
    }

    #[test]
    fn comments_are_not_paths() {
        let img = parse("instance a : A { mount / b # the root\n mount /c#d c }").unwrap();
        let ats: Vec<&str> = img.instances[0].mounts.iter().map(|m| m.at.path.as_str()).collect();
        assert_eq!(ats, ["/", "/c#d"], "`#` only starts a comment at the start of a token");
    }

    #[test]
    fn rejects_malformed_members() {
        let e = parse("instance a : A { mount b }").unwrap_err().to_string();
        assert!(e.contains("expected a path"), "{e}");
        let e = parse("instance a : A { mount b /b }").unwrap_err().to_string();
        assert!(e.contains("the mount point comes first: `mount /b b`"), "{e}");
        let e = parse("mount cons /dev/cons").unwrap_err().to_string();
        assert!(e.contains("the mount point comes first: `mount /dev/cons cons`"), "{e}");
        let e = parse("write hosts").unwrap_err().to_string();
        assert!(e.contains("expected a path"), "{e}");
        let e = parse("instance a : A { frobnicate /x }").unwrap_err().to_string();
        assert!(e.contains("expected `mount`, `include` or `}`"), "{e}");
        let e = parse("instance http : H { control /hosts/* Text }").unwrap_err().to_string();
        assert!(e.contains("requests are made at the top level, in the image's view; mount `http` there, e.g. `mount /http http`, and then `control /http/hosts/* ...`"), "{e}");
        let e = parse("instance net : T { control / listen }").unwrap_err().to_string();
        assert!(e.contains("`control /net ...`"), "{e}");
        let e = parse("frobnicate /x").unwrap_err().to_string();
        assert!(e.contains("or a verb"), "{e}");
    }

    #[test]
    fn rejects_anonymous_messages() {
        let err = parse("write /a { x: 1 }").unwrap_err();
        assert!(err.to_string().contains("anonymous"), "{err}");
    }

    #[test]
    fn reports_position() {
        let err = parse("instance a : A {\n  mount /b b\n  bogus\n}").unwrap_err();
        assert!(err.to_string().starts_with("3:3:"), "{err}");
    }

    #[test]
    fn keywords_are_not_names() {
        assert!(parse("instance instance : P").is_err());
        assert!(parse("instance a.b : P").is_err());
        assert!(parse("instance write : P").is_ok(), "verbs only start a request where a declaration could");
    }
}
