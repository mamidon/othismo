//! The declarations in a `.oth` file, parsed: protocols, traits, objects
//! and what an image needs, the schemas blocks, kept as written, and the
//! message types that BSON's blocks declare, which share the grammar of
//! fields. Resolving them is the compiler's.

use crate::image_definition::{self, Ident, Import, Parser, Pos, Tok};
use anyhow::{Result, bail};
use protocol::ns::Verb;

/// A `.oth` file's declarations: what it imports, declares and exports.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub imports: Vec<Import>,
    pub protocols: Vec<ProtocolDecl>,
    pub traits: Vec<TraitDecl>,
    pub objects: Vec<ObjectDecl>,
    /// What it exports: declarations written `export`, and imported ones
    /// named in `export { ... }`.
    pub exports: Vec<Ident>,
    /// Top-level `needs`: what an image needs from whoever instantiates it.
    pub needs: Vec<NeedDecl>,
    /// `[export] schemas <content type> { ... }`: at most one per content type.
    pub schemas: Vec<SchemasDecl>,
}

/// `[export] schemas <content type> { <body> }`: the types of payloads of one
/// content type, in that content type's format's own language. Othismo
/// reads only as far as the closing brace.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemasDecl {
    /// The content type, as written.
    pub ty: Ident,
    /// Everything between the braces, as written.
    pub body: String,
    /// Where the body starts: just after the `{`.
    pub at: Pos,
    pub exported: bool,
}

impl SchemasDecl {
    /// The body preceded by whitespace, so that its lines and columns are
    /// the file's: what a format parses, so that its errors point into the
    /// file.
    pub fn positioned_body(&self) -> String {
        let mut out = "\n".repeat(self.at.line - 1);
        out.push_str(&" ".repeat(self.at.col - 1));
        out.push_str(&self.body);
        out
    }
}

/// `protocol <Name> { <item>, ... }`: the requests one path takes.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolDecl {
    pub name: Ident,
    pub doc: String,
    pub items: Vec<Item>,
}

/// `trait <Name> { <path> <items> ... }`: the shape of a tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDecl {
    pub name: Ident,
    pub doc: String,
    pub entries: Vec<EntryDecl>,
}

/// What a path takes: a request, or a protocol's or a trait's name. A
/// trait stands alone, and puts its whole tree at the path.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Request(RequestDecl),
    Named(Ident),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectDecl {
    pub name: Ident,
    pub doc: String,
    /// `object <Name> : <Trait>, ...`
    pub traits: Vec<Ident>,
    /// `guide "<file>"`, relative to the header.
    pub guide: Option<(String, Pos)>,
    pub needs: Vec<NeedDecl>,
    pub serves: Vec<EntryDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeedDecl {
    pub at: image_definition::Path,
    pub doc: String,
    pub entries: Vec<EntryDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryDecl {
    pub path: image_definition::Path,
    pub doc: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestDecl {
    pub operator: bool,
    pub verb: Verb,
    pub pos: Pos,
    pub body: Option<Ident>,
    /// The answer's type, and whether it is a stream.
    pub answer: Option<(Ident, bool)>,
}

/// The declarations in a `.oth` file. Anything else in it is an image, which
/// importing its declarations leaves alone.
pub fn parse(src: &str) -> Result<File> {
    Ok(image_definition::parse(src)?.header)
}

impl Parser {
    /// Consecutive `##` lines, joined; empty if there are none.
    pub fn docs(&mut self) -> String {
        let mut lines = vec![];
        while let Tok::Doc(line) = self.peek() {
            lines.push(line.clone());
            self.next();
        }
        lines.join("\n")
    }

    pub fn no_docs(&mut self, doc: &str, pos: Pos) -> Result<()> {
        if !doc.is_empty() {
            bail!("{pos}: a `##` comment documents what follows it, and nothing documentable does");
        }
        Ok(())
    }

    pub fn protocol(&mut self, doc: String) -> Result<ProtocolDecl> {
        self.expect_kw("protocol")?;
        let name = self.ident("protocol name", false)?;
        self.expect(Tok::LBrace)?;
        let mut items = vec![];
        while *self.peek() != Tok::RBrace {
            items.push(self.item()?);
            if *self.peek() == Tok::Comma {
                self.next();
            } else if *self.peek() != Tok::RBrace {
                bail!("{}: expected `,` or `}}`, found {}", self.pos(), self.peek());
            }
        }
        self.next();
        Ok(ProtocolDecl { name, doc, items })
    }

    pub fn trait_decl(&mut self, doc: String) -> Result<TraitDecl> {
        self.expect_kw("trait")?;
        let name = self.ident("trait name", false)?;
        let entries = self.entries()?;
        Ok(TraitDecl { name, doc, entries })
    }

    pub fn object(&mut self, doc: String) -> Result<ObjectDecl> {
        self.expect_kw("object")?;
        let name = self.ident("object name", true)?;
        let mut traits = vec![];
        if *self.peek() == Tok::Colon {
            self.next();
            traits = self.ident_list("trait name")?;
        }
        self.expect(Tok::LBrace)?;
        let mut obj = ObjectDecl { name, doc, traits, guide: None, needs: vec![], serves: vec![] };
        loop {
            let doc = self.docs();
            let (tok, pos) = self.next();
            match tok {
                Tok::RBrace => {
                    self.no_docs(&doc, pos)?;
                    return Ok(obj);
                }
                Tok::Ident(kw) if kw == "needs" => obj.needs.push(self.need(doc)?),
                Tok::Ident(kw) if kw == "serves" => {
                    self.no_docs(&doc, pos)?;
                    obj.serves.extend(self.entries()?);
                }
                Tok::Ident(kw) if kw == "guide" => {
                    self.no_docs(&doc, pos)?;
                    if obj.guide.is_some() {
                        bail!("{pos}: `{}` names its guide twice", obj.name.name);
                    }
                    match self.next() {
                        (Tok::Str(s), _) => obj.guide = Some((s, pos)),
                        (other, pos) => bail!("{pos}: expected the guide's quoted file name, found {other}"),
                    }
                }
                other => bail!("{pos}: expected `needs`, `serves`, `guide` or `}}`, found {other}"),
            }
        }
    }

    /// What follows `needs`: `<mount point> <trait>` for the trait's whole
    /// tree, or `<mount point> { entries }`.
    pub fn need(&mut self, doc: String) -> Result<NeedDecl> {
        let at = self.path()?;
        let entries = match self.peek() {
            Tok::Ident(_) => {
                let name = self.ident("trait name", false)?;
                let path = image_definition::Path { path: "/".into(), pos: name.pos };
                vec![EntryDecl { path, doc: String::new(), items: vec![Item::Named(name)] }]
            }
            _ => self.entries()?,
        };
        Ok(NeedDecl { at, doc, entries })
    }

    /// `{ <path> <request>, ... }`, each path optionally documented.
    fn entries(&mut self) -> Result<Vec<EntryDecl>> {
        self.expect(Tok::LBrace)?;
        let mut entries = vec![];
        loop {
            let doc = self.docs();
            if *self.peek() == Tok::RBrace {
                let pos = self.pos();
                self.no_docs(&doc, pos)?;
                self.next();
                return Ok(entries);
            }
            let path = self.path()?;
            let mut items = vec![self.item()?];
            while *self.peek() == Tok::Comma {
                self.next();
                items.push(self.item()?);
            }
            entries.push(EntryDecl { path, doc, items });
        }
    }

    /// A request, which starts with a verb, or a protocol's or trait's name.
    fn item(&mut self) -> Result<Item> {
        match self.peek() {
            Tok::Ident(w) if w == "control" => bail!(
                "{}: `control` is gone: mark a request only whoever runs the image may make with `operator`, \
                 e.g. `operator write text/plain`",
                self.pos()
            ),
            Tok::Ident(w) if w != OPERATOR && Verb::from_keyword(w).is_none() => {
                Ok(Item::Named(self.ident("protocol or trait name", false)?))
            }
            _ => Ok(Item::Request(self.request()?)),
        }
    }

    /// A content type, or a name standing for one, such as `Stat`.
    fn content_type(&mut self, what: &str) -> Result<Ident> {
        match self.next() {
            (Tok::Type(name), pos) => Ok(Ident { name, pos }),
            (Tok::Ident(name), pos) if !image_definition::KEYWORDS.contains(&name.as_str()) => Ok(Ident { name, pos }),
            (other, pos) => bail!("{pos}: expected {what}, such as `text/plain`, found {other}"),
        }
    }

    fn request(&mut self) -> Result<RequestDecl> {
        let operator = self.is_kw(OPERATOR);
        if operator {
            self.next();
        }
        let (tok, pos) = self.next();
        let verb = match &tok {
            Tok::Ident(w) => Verb::from_keyword(w),
            _ => None,
        };
        let Some(verb) = verb else {
            let verbs: Vec<&str> = protocol::ns::VERBS.iter().map(|v| v.keyword()).collect();
            bail!("{pos}: expected a verb ({}), found {tok}", verbs.join(", "));
        };
        let body = match self.peek() {
            Tok::Type(_) => Some(self.content_type("content type")?),
            Tok::Ident(w) if Verb::from_keyword(w).is_none() => Some(self.ident("content type", false)?),
            _ => None,
        };
        let answer = if *self.peek() == Tok::Arrow {
            self.next();
            let ty = self.content_type("the answer's content type")?;
            let stream = *self.peek() == Tok::Star;
            if stream {
                self.next();
            }
            Some((ty, stream))
        } else {
            None
        };
        Ok(RequestDecl { operator, verb, pos, body, answer })
    }
}

/// Marks a request only whoever runs the image may make.
const OPERATOR: &str = "operator";

