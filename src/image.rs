//! Lexer, AST and parser for `.oth` files, which hold images and headers
//! alike: declarations (message types, protocols, traits and objects, see
//! [`crate::iface`]) and an image's instances, views and requests.

use crate::iface;
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

/// A parsed `.oth` file: its declarations, which other files can import,
/// and the image it describes, which is empty for a header.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// Its imports, declarations, exported declarations and top-level
    /// `needs`.
    pub header: iface::File,
    pub requires: Vec<Require>,
    pub instances: Vec<Instance>,
    /// Top-level mounts: the image's own view, which its requests resolve in.
    pub mounts: Vec<Mount>,
    /// `export <path> [at <path>] [readonly]`: what an instance of the image serves.
    pub exports: Vec<Export>,
    /// `<verb> <path> [T { ... }]`: startup requests, in the order they are made.
    pub requests: Vec<Request>,
}

/// `export <path> [at <path>] [readonly]`
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    /// A path in the image's view.
    pub path: Path,
    /// Where an instance of the image serves it: `path` itself, unless it says.
    pub at: Option<Path>,
    pub read_only: bool,
}

/// `import Image, { A, B as C } from <source>`
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// The file itself, as an image that can be instantiated, by the name
    /// it is given here.
    pub default: Option<Ident>,
    pub names: Vec<ImportName>,
    pub from: Source,
    pub pos: Pos,
}

/// `A`, or `A as B`: a name the source exports, and what it is called here.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportName {
    pub name: Ident,
    pub local: Ident,
}

/// Where an import's declarations come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// `from env`: the runtime's header.
    Env,
    /// `from "./scripts/Greeter.oth"`: a file, relative to the importing
    /// one; a `.oth` file, or a module, whose header is inside it.
    Path(String),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Source::Env => f.write_str("env"),
            Source::Path(p) => write!(f, "{p:?}"),
        }
    }
}

/// `require <name> : <Type> [{ defaults }]` or `require <name> :
/// <content type> [= "<default>"]`: a slot the host fills in.
#[derive(Debug, Clone, PartialEq)]
pub struct Require {
    pub name: Ident,
    pub slot: SlotDecl,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SlotDecl {
    /// A message: its type and any default fields, which need not be all of them.
    Message(Body),
    /// A payload of a content type, such as `text/plain`, given as a string.
    Payload { ty: Ident, default: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub name: Ident,
    /// An object, one of the runtime's objects, or an image, as imported.
    pub type_name: Ident,
    pub mounts: Vec<Mount>,
    /// `bind <path> <path in the image's view> [readonly]`
    pub binds: Vec<Bind>,
    /// `<slot> = <value>`: an instantiated image's slots.
    pub values: Vec<(Ident, Literal)>,
    /// `include "<path>" [at <path>]`: files to pack into the image.
    pub includes: Vec<Include>,
}

/// `bind <at> <path> [readonly]`: what `path` means in the image's own
/// view, at `at` in the instance's.
#[derive(Debug, Clone, PartialEq)]
pub struct Bind {
    pub at: Path,
    pub path: Path,
    pub read_only: bool,
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
    pub body: Option<Literal>,
}

/// What a startup request carries.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// A message literal, or a slot's name.
    Message(Body),
    /// `"text"`, a `text/plain` payload, or `<content type> "text"`.
    Text { ty: Option<Ident>, text: String, pos: Pos },
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
    /// `=`, before a slot's default.
    Eq,
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
            Tok::Eq => write!(f, "`=`"),
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
        } else if c == '=' {
            toks.push((Tok::Eq, pos));
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
/// `readonly` and `at`) only mean something inside an instance block, `as`
/// only in an import, and the verbs only where a request can start.
pub(crate) const KEYWORDS: &[&str] = &["import", "export", "from", "require", "instance", "true", "false"];

/// Words that start a declaration at the top of a file, besides the verbs.
const DECLS: &[&str] = &["import", "export", "require", "instance", "mount", "message", "protocol", "trait", "object"];

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

    fn file(&mut self) -> Result<Image> {
        let header = iface::File {
            imports: vec![],
            messages: vec![],
            protocols: vec![],
            traits: vec![],
            objects: vec![],
            exports: vec![],
            needs: vec![],
        };
        let mut image =
            Image { header, requires: vec![], instances: vec![], mounts: vec![], exports: vec![], requests: vec![] };
        loop {
            let doc = self.docs();
            let pos = self.pos();
            let documented =
                matches!(self.peek(), Tok::Ident(kw) if ["export", "message", "protocol", "trait", "object", "needs"].contains(&kw.as_str()));
            if !documented {
                self.no_docs(&doc, pos)?;
            }
            match self.peek().clone() {
                Tok::Eof => return Ok(image),
                Tok::Ident(kw) if kw == "import" && matches!(self.toks.get(self.i + 1), Some((Tok::Str(_), _))) => bail!(
                    "{pos}: an image is no longer imported whole: import it by a name, `import Site from \"...\"`, \
                     and make an instance of it, `instance site : Site {{ ... }}`"
                ),
                Tok::Ident(kw) if kw == "import" => image.header.imports.push(self.import()?),
                Tok::Ident(kw) if kw == "needs" => {
                    self.next();
                    image.header.needs.push(self.need(doc)?);
                }
                Tok::Ident(kw) if kw == "export" && matches!(self.toks.get(self.i + 1), Some((Tok::Path(_), _))) => {
                    self.next();
                    let path = self.path()?;
                    let at = if self.is_kw("at") {
                        self.next();
                        Some(self.path()?)
                    } else {
                        None
                    };
                    let read_only = self.is_kw("readonly");
                    if read_only {
                        self.next();
                    }
                    image.exports.push(Export { path, at, read_only });
                }
                Tok::Ident(kw) if kw == "export" => {
                    self.next();
                    if *self.peek() == Tok::LBrace {
                        self.no_docs(&doc, pos)?;
                        self.next();
                        image.header.exports.extend(self.ident_list("name to export")?);
                        self.expect(Tok::RBrace)?;
                    } else {
                        let name = self.declaration(&mut image.header, doc, pos)?;
                        image.header.exports.push(name);
                    }
                }
                Tok::Ident(kw) if ["message", "protocol", "trait", "object"].contains(&kw.as_str()) => {
                    self.declaration(&mut image.header, doc, pos)?;
                }
                Tok::Ident(kw) if kw == "instance" => image.instances.push(self.instance()?),
                Tok::Ident(kw) if kw == "require" => image.requires.push(self.require()?),
                Tok::Ident(kw) if kw == "mount" => {
                    self.next();
                    image.mounts.push(self.mount()?);
                }
                Tok::Ident(kw) if Verb::from_keyword(&kw).is_some() => image.requests.push(self.startup_request()?),
                Tok::Ident(kw) if kw == "control" => bail!(
                    "{}: `control` is gone: the image's requests are all the operator's, so write the request with its \
                     own verb, such as `write`",
                    self.pos()
                ),
                other => bail!(
                    "{pos}: expected `import`, `export`, `message`, `protocol`, `trait`, `object`, `needs`, `require`, \
                     `instance`, `mount` or a verb, found {other}"
                ),
            }
        }
    }

    /// A message type, protocol, trait or object, added to `header`; its name.
    fn declaration(&mut self, header: &mut iface::File, doc: String, pos: Pos) -> Result<Ident> {
        Ok(match self.peek().clone() {
            Tok::Ident(kw) if kw == "message" => {
                let m = self.message(doc)?;
                let name = m.name.clone();
                header.messages.push(m);
                name
            }
            Tok::Ident(kw) if kw == "protocol" => {
                let p = self.protocol(doc)?;
                let name = p.name.clone();
                header.protocols.push(p);
                name
            }
            Tok::Ident(kw) if kw == "trait" => {
                let t = self.trait_decl(doc)?;
                let name = t.name.clone();
                header.traits.push(t);
                name
            }
            Tok::Ident(kw) if kw == "object" => {
                let o = self.object(doc)?;
                let name = o.name.clone();
                header.objects.push(o);
                name
            }
            other => bail!("{pos}: `export` comes before a message, protocol, trait or object, or `{{ names }}`, found {other}"),
        })
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
            Tok::Ident(w) if DECLS.contains(&w.as_str()) || Verb::from_keyword(w).is_some() => None,
            Tok::Ident(_) | Tok::LBrace | Tok::Str(_) | Tok::Type(_) => Some(self.literal()?),
            _ => None,
        };
        Ok(Request { verb, verb_pos, path, body })
    }

    /// A message literal or a slot's name, `"text"`, or `<content type> "text"`.
    fn literal(&mut self) -> Result<Literal> {
        Ok(match self.peek().clone() {
            Tok::Str(text) => {
                let (_, pos) = self.next();
                Literal::Text { ty: None, text, pos }
            }
            Tok::Type(ty) => {
                let (_, pos) = self.next();
                match self.next() {
                    (Tok::Str(text), _) => Literal::Text { ty: Some(Ident { name: ty, pos }), text, pos },
                    (other, pos) => bail!("{pos}: expected a quoted string after `{ty}`, found {other}"),
                }
            }
            _ => Literal::Message(self.body()?),
        })
    }

    /// `import { A, B as C } from env`, or `import Image, { ... } from
    /// "./file.oth"`, or `import Image from "./file.oth"`.
    pub(crate) fn import(&mut self) -> Result<Import> {
        let pos = self.expect_kw("import")?;
        let mut default = None;
        if let Tok::Ident(_) = self.peek() {
            default = Some(self.ident("a name for the image", false)?);
            if *self.peek() != Tok::Comma {
                self.expect_kw("from")?;
                let from = match self.next() {
                    (Tok::Str(s), _) => Source::Path(s),
                    (other, pos) => bail!("{pos}: an image is imported from its file, quoted, found {other}"),
                };
                return Ok(Import { default, names: vec![], from, pos });
            }
            self.next();
        }
        self.expect(Tok::LBrace)?;
        let mut names = vec![];
        loop {
            let name = self.ident("name to import", true)?;
            let local = if self.is_kw("as") {
                self.next();
                self.ident("name to import it as", false)?
            } else {
                name.clone()
            };
            names.push(ImportName { name, local });
            if *self.peek() != Tok::Comma {
                break;
            }
            self.next();
        }
        self.expect(Tok::RBrace)?;
        self.expect_kw("from")?;
        let from = match self.next() {
            (Tok::Str(s), _) => Source::Path(s),
            (Tok::Ident(name), _) if name == "env" => Source::Env,
            (Tok::Ident(name), pos) if !KEYWORDS.contains(&name.as_str()) => bail!(
                "{pos}: import from a file, relative to this one, such as `from \"./scripts/{name}.oth\"`, or from env"
            ),
            (other, pos) => bail!("{pos}: expected env or a quoted path to import from, found {other}"),
        };
        if default.is_some() && from == Source::Env {
            bail!("{pos}: env has no image to import, only declarations");
        }
        Ok(Import { default, names, from, pos })
    }

    fn require(&mut self) -> Result<Require> {
        self.expect_kw("require")?;
        let name = self.ident("slot name", false)?;
        self.expect(Tok::Colon)?;
        let Tok::Type(ty) = self.peek().clone() else { return Ok(Require { name, slot: SlotDecl::Message(self.body()?) }) };
        let (_, pos) = self.next();
        let mut default = None;
        if *self.peek() == Tok::Eq {
            self.next();
            match self.next() {
                (Tok::Str(s), _) => default = Some(s),
                (other, pos) => bail!("{pos}: expected the slot's default, a quoted string, found {other}"),
            }
        }
        Ok(Require { name, slot: SlotDecl::Payload { ty: Ident { name: ty, pos }, default } })
    }

    fn instance(&mut self) -> Result<Instance> {
        self.expect_kw("instance")?;
        let name = self.ident("instance name", false)?;
        self.expect(Tok::Colon)?;
        let type_name = self.ident("instance type", true)?;
        let mut inst = Instance { name, type_name, mounts: vec![], binds: vec![], values: vec![], includes: vec![] };
        if *self.peek() != Tok::LBrace {
            return Ok(inst);
        }
        self.next();
        loop {
            let (tok, pos) = self.next();
            let word = match tok {
                Tok::RBrace => return Ok(inst),
                Tok::Ident(word) => word,
                other => bail!("{pos}: expected `mount`, `bind`, `include`, a slot's value or `}}`, found {other}"),
            };
            if *self.peek() == Tok::Eq {
                self.next();
                inst.values.push((Ident { name: word, pos }, self.literal()?));
            } else if word == "mount" {
                inst.mounts.push(self.mount()?);
            } else if word == "bind" {
                let at = self.path()?;
                let path = self.path()?;
                let read_only = self.is_kw("readonly");
                if read_only {
                    self.next();
                }
                inst.binds.push(Bind { at, path, read_only });
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
                bail!("{pos}: expected `mount`, `bind`, `include`, a slot's value or `}}`, found `{word}`");
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

    /// A value file: `slot { field: value, ... }` or `slot = "text"`, any
    /// number of times.
    fn values(&mut self) -> Result<Vec<SlotValues>> {
        let mut out = vec![];
        while *self.peek() != Tok::Eof {
            let slot = self.ident("slot name", false)?;
            let value = if *self.peek() == Tok::Eq {
                self.next();
                match self.next() {
                    (Tok::Str(s), pos) => SlotValue::Text(s, pos),
                    (other, pos) => bail!("{pos}: expected a quoted string, found {other}"),
                }
            } else {
                SlotValue::Fields(self.fields()?)
            };
            out.push(SlotValues { slot, value });
        }
        Ok(out)
    }
}

pub fn parse(src: &str) -> Result<Image> {
    Parser::new(src, true)?.file()
}

/// One slot's values in a value file.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotValues {
    pub slot: Ident,
    pub value: SlotValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SlotValue {
    /// `slot { field: value, ... }`, for a message.
    Fields(Vec<(Ident, Value)>),
    /// `slot = "text"`, for a payload.
    Text(String, Pos),
}

/// Parses a value file, which gives slots their values in the image's own
/// syntax: `listen = "0.0.0.0:80"`, `db { port: 5432 }`.
pub fn parse_values(src: &str) -> Result<Vec<SlotValues>> {
    Parser::new(src, false)?.values()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ids: &[ImportName]) -> Vec<String> {
        ids.iter().map(|i| if i.name == i.local { i.name.name.clone() } else { format!("{} as {}", i.name.name, i.local.name) }).collect()
    }

    #[test]
    fn parses_full_syntax() {
        let img = parse(
            r#"
            import { Greeting, Ping as Pong } from "./messages.oth"   # trailing comment
            import { Tick } from "./other.oth"

            instance cons : Console
            instance net : Listener
            write /net Listen { address: "127.0.0.1:0" }
            instance echo : Echo {
              mount /dev/cons cons
              mount /net/tcp net readonly
            }
            write /echo/greeting Greeting { prefix: "hi \"there\"", n: -3, x: 1.5, ok: true }
            create /echo Ping
            remove /echo/old
            watch /echo
            "#,
        )
        .unwrap();

        let imports = &img.header.imports;
        assert_eq!(imports.len(), 2);
        assert_eq!(names(&imports[0].names), ["Greeting", "Ping as Pong"]);
        assert_eq!(imports[0].from, Source::Path("./messages.oth".into()));

        assert_eq!(img.instances.len(), 3);
        assert_eq!(img.instances[0].type_name.name, "Console");
        let echo = &img.instances[2];
        let mounts: Vec<_> = echo.mounts.iter().map(|m| (m.target.name.as_str(), m.at.path.as_str(), m.read_only)).collect();
        assert_eq!(mounts, [("cons", "/dev/cons", false), ("net", "/net/tcp", true)]);

        let reqs = &img.requests;
        let verbs: Vec<_> = reqs.iter().map(|r| r.verb).collect();
        assert_eq!(verbs, [Verb::Write, Verb::Write, Verb::Create, Verb::Remove, Verb::Watch]);
        let msg = |r: &Request| match r.body.as_ref().unwrap() {
            Literal::Message(b) => b.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(reqs[0].path.path, "/net");
        assert_eq!(msg(&reqs[0]).type_name.name, "Listen");
        assert_eq!(reqs[1].path.path, "/echo/greeting");
        let f = msg(&reqs[1]).fields.unwrap();
        assert_eq!(f[0].1, Value::Str("hi \"there\"".into()));
        assert_eq!(f[1].1, Value::Int(-3));
        assert_eq!(f[2].1, Value::Float(1.5));
        assert_eq!(f[3].1, Value::Bool(true));
        assert_eq!(reqs[2].path.path, "/echo");
        assert_eq!(msg(&reqs[2]).fields, None);
        assert_eq!(reqs[3].body, None, "a verb ends a body-less request");
        assert_eq!(reqs[4].body, None);
    }

    #[test]
    fn text_literals_are_payloads() {
        let img = parse(r#"write /a "hi" write /b text/html "<p>" remove /c"#).unwrap();
        let bodies: Vec<_> = img.requests.iter().map(|r| r.body.clone()).collect();
        assert_eq!(bodies[0], Some(Literal::Text { ty: None, text: "hi".into(), pos: Pos { line: 1, col: 10 } }));
        let Some(Literal::Text { ty: Some(ty), text, .. }) = &bodies[1] else { panic!("{:?}", bodies[1]) };
        assert_eq!((ty.name.as_str(), text.as_str()), ("text/html", "<p>"));
        assert_eq!(bodies[2], None);
        assert!(parse("write /a text/plain 3").unwrap_err().to_string().contains("expected a quoted string after `text/plain`"));
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
            write /net listen"#,
        )
        .unwrap();
        let msg = |r: &Require| match &r.slot {
            SlotDecl::Message(b) => b.clone(),
            other => panic!("{other:?}"),
        };
        let reqs: Vec<_> = img.requires.iter().map(|r| (r.name.name.as_str(), msg(r).type_name.name)).collect();
        assert_eq!(reqs, [("listen", "Listen".to_string()), ("admin", "Text".to_string())]);
        assert_eq!(msg(&img.requires[0]).fields.as_ref().unwrap()[0].1, Value::Str("127.0.0.1:8080".into()));
        assert_eq!(msg(&img.requires[1]).fields, None);
        let Some(Literal::Message(body)) = img.requests[0].body.as_ref() else { panic!() };
        assert_eq!(body.type_name.name, "listen", "a slot is used like a unit message");
        assert!(parse("require : Text").is_err());
        assert!(parse("instance require : P").is_err(), "`require` is a keyword");

        let img = parse(r#"require a : text/plain = "x" require b : text/html"#).unwrap();
        let slots: Vec<_> = img
            .requires
            .iter()
            .map(|r| match &r.slot {
                SlotDecl::Payload { ty, default } => (ty.name.clone(), default.clone()),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(slots, [("text/plain".to_string(), Some("x".to_string())), ("text/html".into(), None)]);
        assert!(parse("require a : text/plain = 3").unwrap_err().to_string().contains("a quoted string"));
    }

    #[test]
    fn parses_value_files() {
        let v = parse_values("# prod\nlisten { address: \"0.0.0.0:80\" }\ndb { port: 5432, tls: true }\nhost = \"x\"").unwrap();
        assert_eq!(v.len(), 3);
        let SlotValue::Fields(listen) = &v[0].value else { panic!() };
        assert_eq!((v[0].slot.name.as_str(), listen[0].0.pos), ("listen", Pos { line: 2, col: 10 }));
        let SlotValue::Fields(db) = &v[1].value else { panic!() };
        assert_eq!(db[1].1, Value::Bool(true));
        assert_eq!(v[2].value, SlotValue::Text("x".into(), Pos { line: 4, col: 8 }));
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
    fn imports_name_env_or_a_path() {
        let img = parse("import { Console } from env\nimport { Greeting } from \"./scripts/Greeter.oth\"").unwrap();
        let froms: Vec<String> = img.header.imports.iter().map(|i| i.from.to_string()).collect();
        assert_eq!(froms, ["env", "\"./scripts/Greeter.oth\""]);
        assert!(parse("import { Line } from instance").is_err(), "a keyword isn't a source");
        let e = parse("import { Greeting } from Greeter").unwrap_err().to_string();
        assert!(e.contains("such as `from \"./scripts/Greeter.oth\"`"), "{e}");
    }

    #[test]
    fn one_file_declares_and_instantiates() {
        let img = parse(
            "## A greeting.\nexport message Greeting { text: string }\nprotocol P { read }\nexport { P }\n\
             ## An object.\nexport object O {}\ninstance o : O\nwrite /o Greeting { text: \"hi\" }",
        )
        .unwrap();
        let h = &img.header;
        assert_eq!((h.messages.len(), h.protocols.len(), h.objects.len()), (1, 1, 1));
        assert_eq!(h.messages[0].doc, "A greeting.");
        assert_eq!(h.objects[0].doc, "An object.");
        let exports: Vec<&str> = h.exports.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(exports, ["Greeting", "P", "O"]);
        assert_eq!((img.instances.len(), img.requests.len()), (1, 1));
        let e = parse("## an instance\ninstance o : O").unwrap_err().to_string();
        assert!(e.contains("documents what follows"), "{e}");
        assert!(parse("export instance o : O").unwrap_err().to_string().contains("`export` comes before"));
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
        assert!(e.contains("expected `mount`, `bind`, `include`, a slot's value or `}`"), "{e}");
        let e = parse("instance http : H { write /hosts/* Text }").unwrap_err().to_string();
        assert!(e.contains("requests are made at the top level, in the image's view; mount `http` there, e.g. `mount /http http`, and then `write /http/hosts/* ...`"), "{e}");
        let e = parse("instance net : T { write / listen }").unwrap_err().to_string();
        assert!(e.contains("`write /net ...`"), "{e}");
        let e = parse("control /net listen").unwrap_err().to_string();
        assert!(e.contains("`control` is gone"), "{e}");
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
