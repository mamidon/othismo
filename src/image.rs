//! Lexer, AST and parser for the image language (`.img` files).

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
    pub protocols: Vec<Protocol>,
    pub instances: Vec<Instance>,
    /// Top-level routing trees, in source order.
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub names: Vec<Ident>,
    pub path: String,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Protocol {
    pub name: Ident,
    /// Types the object accepts, whether routed to it or replied to it.
    pub inputs: Vec<Ident>,
    /// Types the object may `emit`; the image routes them.
    pub emits: Vec<Ident>,
    /// Types the object may `reply` with, back to whoever messaged it.
    pub replies: Vec<Ident>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub name: Ident,
    /// Protocol name; may be dotted, e.g. `Environment.Stdin`.
    pub protocol: Ident,
    pub sends: Vec<Send>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Send {
    pub type_name: Ident,
    /// `None` for a unit message (`send TypeName`).
    pub fields: Option<Vec<(Ident, Value)>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i128),
    Float(f64),
    Bool(bool),
}

/// `sender.Type { targets }`: what `sender` emits of `Type` goes to every target.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub sender: Ident,
    /// A message type name, or `*` for every type the sender emits.
    pub selector: Ident,
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Just an instance name.
    Leaf(Ident),
    /// An instance that also routes some of what it emits: the nested
    /// route's sender is the target.
    Node(Route),
}

impl Target {
    pub fn instance(&self) -> &Ident {
        match self {
            Target::Leaf(id) => id,
            Target::Node(r) => &r.sender,
        }
    }
}

// ---------------------------------------------------------------------------
// Lexer

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
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
                // `name.*` selects every type in a route.
                if chars[i] == '.' && chars.get(i + 1) == Some(&'*') {
                    bump!();
                    bump!();
                    break;
                }
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

/// Words that can never be names. `in` and `out` are only meaningful at the
/// start of a protocol member, so ports may be called `out` (as in the spec).
const KEYWORDS: &[&str] = &["import", "from", "protocol", "instance", "send", "true", "false"];

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

    fn image(&mut self) -> Result<Image> {
        let mut image = Image { imports: vec![], protocols: vec![], instances: vec![], routes: vec![] };
        loop {
            match self.peek().clone() {
                Tok::Eof => return Ok(image),
                Tok::Ident(kw) if kw == "import" => image.imports.push(self.import()?),
                Tok::Ident(kw) if kw == "protocol" => image.protocols.push(self.protocol()?),
                Tok::Ident(kw) if kw == "instance" => image.instances.push(self.instance()?),
                Tok::Ident(name) if name.contains('.') => image.routes.push(self.route()?),
                other => bail!(
                    "{}: expected `import`, `protocol`, `instance` or a route (`sender.Type {{ ... }}`), found {other}",
                    self.pos()
                ),
            }
        }
    }

    /// `sender.Type { target, other.Type { ... }, ... }`
    fn route(&mut self) -> Result<Route> {
        let (tok, pos) = self.next();
        let Tok::Ident(name) = tok else { bail!("{pos}: expected a route, found {tok}") };
        let Some((sender, selector)) = name.split_once('.').filter(|(a, b)| {
            !a.is_empty() && !b.is_empty() && !b.contains('.') && !KEYWORDS.contains(a) && !KEYWORDS.contains(b)
        }) else {
            bail!("{pos}: expected `instance.Type` or `instance.*`, found `{name}`");
        };
        let sender = Ident { name: sender.into(), pos };
        let selector = Ident { name: selector.into(), pos: Pos { line: pos.line, col: pos.col + sender.name.len() + 1 } };
        self.expect(Tok::LBrace)?;
        let mut targets = vec![];
        while *self.peek() != Tok::RBrace {
            match self.peek().clone() {
                Tok::Ident(n) if n.contains('.') => targets.push(Target::Node(self.route()?)),
                _ => targets.push(Target::Leaf(self.ident("target instance or `instance.Type { ... }`", false)?)),
            }
            if *self.peek() == Tok::Comma {
                self.next();
            }
        }
        self.next();
        if targets.is_empty() {
            bail!("{pos}: route `{name}` has no targets");
        }
        Ok(Route { sender, selector, targets })
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

    fn protocol(&mut self) -> Result<Protocol> {
        self.expect_kw("protocol")?;
        let name = self.ident("protocol name", false)?;
        self.expect(Tok::LBrace)?;
        let mut proto = Protocol { name, inputs: vec![], emits: vec![], replies: vec![] };
        loop {
            let kw = match self.peek() {
                Tok::RBrace => {
                    self.next();
                    return Ok(proto);
                }
                Tok::Ident(kw) if matches!(kw.as_str(), "in" | "emits" | "replies") => kw.clone(),
                other => bail!("{}: expected `in`, `emits`, `replies` or `}}`, found {other}", self.pos()),
            };
            self.next();
            let types = self.ident_list("message type")?;
            match kw.as_str() {
                "in" => proto.inputs.extend(types),
                "emits" => proto.emits.extend(types),
                _ => proto.replies.extend(types),
            }
        }
    }

    fn instance(&mut self) -> Result<Instance> {
        self.expect_kw("instance")?;
        let name = self.ident("instance name", false)?;
        self.expect(Tok::Colon)?;
        let protocol = self.ident("protocol name", true)?;
        let mut inst = Instance { name, protocol, sends: vec![] };
        if *self.peek() != Tok::LBrace {
            return Ok(inst);
        }
        self.next();
        loop {
            if *self.peek() == Tok::RBrace {
                self.next();
                return Ok(inst);
            }
            if !self.is_kw("send") {
                bail!(
                    "{}: expected `send` or `}}`, found {}; routes are written at the top level as `{}.Type {{ ... }}`",
                    self.pos(),
                    self.peek(),
                    inst.name.name
                );
            }
            self.next();
            inst.sends.push(self.send()?);
        }
    }

    fn send(&mut self) -> Result<Send> {
        if *self.peek() == Tok::LBrace {
            bail!("{}: anonymous sends are not supported; write `send TypeName {{ ... }}`", self.pos());
        }
        let type_name = self.ident("message type", false)?;
        if *self.peek() != Tok::LBrace {
            return Ok(Send { type_name, fields: None });
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
        Ok(Send { type_name, fields: Some(fields) })
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
            import { Line, Echoed } from "./messages"   // trailing comment
            import { Tick } from "./other"

            protocol Echo {
              in Line, Tick
              emits Echoed, Line
              replies Tick
            }

            instance stdin : Environment.Stdin
            instance echo : Echo {
              send Tick
              send Line { text: "hi \"there\"" }
              send Tick { n: -3, x: 1.5, ok: true }
            }
            instance stdout : Environment.Stdout

            stdin.Line {
              echo.Echoed { stdout, other }
              echo.* { stdout }, extra
            }
            "#,
        )
        .unwrap();

        assert_eq!(img.imports.len(), 2);
        assert_eq!(names(&img.imports[0].names), ["Line", "Echoed"]);
        assert_eq!(img.imports[0].path, "./messages");

        let p = &img.protocols[0];
        assert_eq!(p.name.name, "Echo");
        assert_eq!(names(&p.inputs), ["Line", "Tick"]);
        assert_eq!(names(&p.emits), ["Echoed", "Line"]);
        assert_eq!(names(&p.replies), ["Tick"]);

        assert_eq!(img.instances.len(), 3);
        assert_eq!(img.instances[0].protocol.name, "Environment.Stdin");
        let echo = &img.instances[1];
        assert_eq!(echo.sends[0].fields, None);
        assert_eq!(echo.sends[1].fields.as_ref().unwrap()[0].1, Value::Str("hi \"there\"".into()));
        let f = echo.sends[2].fields.as_ref().unwrap();
        assert_eq!(f[0].1, Value::Int(-3));
        assert_eq!(f[1].1, Value::Float(1.5));
        assert_eq!(f[2].1, Value::Bool(true));

        let root = &img.routes[0];
        assert_eq!((root.sender.name.as_str(), root.selector.name.as_str()), ("stdin", "Line"));
        assert_eq!(root.selector.pos.col, root.sender.pos.col + 6);
        let kids: Vec<&str> = root.targets.iter().map(|t| t.instance().name.as_str()).collect();
        assert_eq!(kids, ["echo", "echo", "extra"]);
        let Target::Node(first) = &root.targets[0] else { panic!() };
        assert_eq!(first.selector.name, "Echoed");
        let leaves: Vec<&str> = first.targets.iter().map(|t| t.instance().name.as_str()).collect();
        assert_eq!(leaves, ["stdout", "other"]);
        let Target::Node(star) = &root.targets[1] else { panic!() };
        assert_eq!(star.selector.name, "*");
        assert!(matches!(root.targets[2], Target::Leaf(_)));
    }

    #[test]
    fn rejects_malformed_routes() {
        assert!(parse("a.b.c { x }").is_err());
        assert!(parse("a.T { }").unwrap_err().to_string().contains("no targets"));
        assert!(parse("a.T { b.U }").is_err(), "nested nodes need braces");
        let e = parse("instance a : P { out }").unwrap_err().to_string();
        assert!(e.contains("routes are written at the top level"), "{e}");
    }

    #[test]
    fn rejects_anonymous_send() {
        let err = parse("instance a : P { send { x: 1 } }").unwrap_err();
        assert!(err.to_string().contains("anonymous"), "{err}");
    }

    #[test]
    fn reports_position() {
        let err = parse("protocol P {\n  in A\n  bogus\n}").unwrap_err();
        assert!(err.to_string().starts_with("3:3:"), "{err}");
    }

    #[test]
    fn keywords_are_not_names() {
        assert!(parse("instance send : P").is_err());
        assert!(parse("protocol Environment.X {}").is_err());
    }
}
