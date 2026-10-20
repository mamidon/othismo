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
    pub inputs: Vec<Ident>,
    pub outputs: Vec<OutPort>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutPort {
    pub name: Ident,
    pub types: Vec<Ident>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub name: Ident,
    /// Protocol name; may be dotted, e.g. `Environment.Stdin`.
    pub protocol: Ident,
    pub sends: Vec<Send>,
    pub wires: Vec<Wire>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct Wire {
    pub port: Ident,
    pub targets: Vec<Ident>,
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
    Arrow,
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
            Tok::Arrow => write!(f, "`->`"),
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
        } else if c == '-' && chars.get(i + 1) == Some(&'>') {
            toks.push((Tok::Arrow, pos));
            bump!();
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
        let mut image = Image { imports: vec![], protocols: vec![], instances: vec![] };
        loop {
            match self.peek().clone() {
                Tok::Eof => return Ok(image),
                Tok::Ident(kw) if kw == "import" => image.imports.push(self.import()?),
                Tok::Ident(kw) if kw == "protocol" => image.protocols.push(self.protocol()?),
                Tok::Ident(kw) if kw == "instance" => image.instances.push(self.instance()?),
                other => bail!("{}: expected `import`, `protocol` or `instance`, found {other}", self.pos()),
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

    fn protocol(&mut self) -> Result<Protocol> {
        self.expect_kw("protocol")?;
        let name = self.ident("protocol name", false)?;
        self.expect(Tok::LBrace)?;
        let mut proto = Protocol { name, inputs: vec![], outputs: vec![] };
        loop {
            if *self.peek() == Tok::RBrace {
                self.next();
                return Ok(proto);
            } else if self.is_kw("in") {
                self.next();
                proto.inputs.extend(self.ident_list("message type")?);
            } else if self.is_kw("out") {
                self.next();
                let name = self.ident("port name", false)?;
                self.expect(Tok::Colon)?;
                let types = self.ident_list("message type")?;
                proto.outputs.push(OutPort { name, types });
            } else {
                bail!("{}: expected `in`, `out` or `}}`, found {}", self.pos(), self.peek());
            }
        }
    }

    fn instance(&mut self) -> Result<Instance> {
        self.expect_kw("instance")?;
        let name = self.ident("instance name", false)?;
        self.expect(Tok::Colon)?;
        let protocol = self.ident("protocol name", true)?;
        let mut inst = Instance { name, protocol, sends: vec![], wires: vec![] };
        if *self.peek() != Tok::LBrace {
            return Ok(inst);
        }
        self.next();
        loop {
            if *self.peek() == Tok::RBrace {
                self.next();
                return Ok(inst);
            } else if self.is_kw("send") {
                self.next();
                inst.sends.push(self.send()?);
            } else {
                let port = self.ident("`send`, port name or `}`", false)?;
                self.expect(Tok::Arrow)?;
                let targets = self.ident_list("target instance")?;
                inst.wires.push(Wire { port, targets });
            }
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
              out out: Echoed
              out log: Line, Echoed
            }

            instance stdin : Environment.Stdin { out -> echo }
            instance echo : Echo {
              send Tick
              send Line { text: "hi \"there\"" }
              send Tick { n: -3, x: 1.5, ok: true }
              out -> stdout
              log -> stdout, other
            }
            instance stdout : Environment.Stdout
            "#,
        )
        .unwrap();

        assert_eq!(img.imports.len(), 2);
        assert_eq!(names(&img.imports[0].names), ["Line", "Echoed"]);
        assert_eq!(img.imports[0].path, "./messages");

        let p = &img.protocols[0];
        assert_eq!(p.name.name, "Echo");
        assert_eq!(names(&p.inputs), ["Line", "Tick"]);
        assert_eq!(p.outputs[1].name.name, "log");
        assert_eq!(names(&p.outputs[1].types), ["Line", "Echoed"]);

        assert_eq!(img.instances.len(), 3);
        assert_eq!(img.instances[0].protocol.name, "Environment.Stdin");
        let echo = &img.instances[1];
        assert_eq!(echo.sends[0], Send { type_name: echo.sends[0].type_name.clone(), fields: None });
        assert_eq!(
            echo.sends[1].fields.as_ref().unwrap()[0].1,
            Value::Str("hi \"there\"".into())
        );
        let f = echo.sends[2].fields.as_ref().unwrap();
        assert_eq!(f[0].1, Value::Int(-3));
        assert_eq!(f[1].1, Value::Float(1.5));
        assert_eq!(f[2].1, Value::Bool(true));
        assert_eq!(names(&echo.wires[1].targets), ["stdout", "other"]);
        assert!(img.instances[2].wires.is_empty());
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
