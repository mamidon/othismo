//! Headers: `.iface` files declaring a module's message types, what it
//! serves, what it needs and its documentation. See `docs/objects.md`.
//!
//! A header is parsed into a [`File`], then resolved into a [`Header`]: its
//! imports followed, every type it mentions declared, and its guide read in.
//! A resolved header stands on its own, so it can travel inside a module.

use crate::image::{self, Ident, Import, Parser, Pos, Source, Tok};
use crate::messages::{FieldType, MessageType};
use crate::ns::{Status, Verb};
use crate::pattern::{self, Pattern};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;

/// The reserved type that stands for any message.
pub const OPAQUE: &str = "Opaque";

/// The runtime's own header: its message types and the built-in objects.
const ENV_SOURCE: &str = include_str!("../as/env.iface");

/// The runtime's header, `env`.
pub fn env() -> &'static Header {
    static ENV: OnceLock<Header> = OnceLock::new();
    ENV.get_or_init(|| {
        let file = parse(ENV_SOURCE).expect("env.iface parses");
        let mut no_imports = |s: &Source| -> Result<Header> { bail!("env.iface can't import from {s}") };
        resolve(&file, &mut no_imports, &|_| bail!("env.iface has no guides")).expect("env.iface resolves")
    })
}

// ---------------------------------------------------------------------------
// Resolved

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// Every message type the header can name: its own and its imports.
    pub messages: BTreeMap<String, MessageType>,
    pub objects: Vec<Object>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub name: String,
    /// The object's `##` comment, Markdown.
    pub doc: String,
    /// The guide's Markdown, if the object names one.
    pub guide: Option<String>,
    pub needs: Vec<Need>,
    pub serves: Vec<Entry>,
}

/// `needs <mount point pattern> { entries }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Need {
    pub at: Pattern,
    pub doc: String,
    pub entries: Vec<Entry>,
}

/// A path pattern and the requests it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub path: Pattern,
    pub doc: String,
    pub requests: Vec<Request>,
}

/// `<verb> [Body] [-> Answer[*]]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub verb: Verb,
    pub body: Option<String>,
    /// `None`: answered with a status alone.
    pub answer: Option<Answer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub ty: String,
    /// `Type*`: any number of answers, the last one marked done.
    pub stream: bool,
}

impl Header {
    pub fn object(&self, name: &str) -> Option<&Object> {
        self.objects.iter().find(|o| o.name == name)
    }
}

impl Object {
    /// The entry serving `path`, a clean path in the object's served tree.
    pub fn serving(&self, path: &str) -> Option<&Entry> {
        pattern::best(&self.serves, |e| &e.path, path)
    }

    /// The `needs` block for the mount point `at`.
    pub fn need(&self, at: &str) -> Option<&Need> {
        pattern::best(&self.needs, |n| &n.at, at)
    }
}

impl Need {
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        pattern::best(&self.entries, |e| &e.path, path)
    }
}

impl Entry {
    /// The request a `verb` with a body of type `body` fits: `NotSupported`
    /// if the entry doesn't take the verb, and `Invalid` if it does but not
    /// with that body.
    pub fn takes(&self, verb: Verb, body: Option<&str>) -> Result<&Request, Status> {
        let mut with_verb = self.requests.iter().filter(|r| r.verb == verb).peekable();
        if with_verb.peek().is_none() {
            return Err(Status::NotSupported);
        }
        with_verb.find(|r| body_fits(r.body.as_deref(), body)).ok_or(Status::Invalid)
    }
}

/// Whether a body of type `given` fits where `want` is declared.
pub fn body_fits(want: Option<&str>, given: Option<&str>) -> bool {
    match (want, given) {
        (None, None) => true,
        (Some(w), Some(g)) => w == OPAQUE || w == g,
        _ => false,
    }
}

impl Request {
    /// Whether this request, as a server serves it, meets `need`: the same
    /// verb and body, and an answer of the same type. `Opaque` goes both
    /// ways: a server taking `Opaque` takes any body, and a need for an
    /// `Opaque` answer accepts any answer, or none.
    pub fn meets(&self, need: &Request) -> bool {
        let body = match (&self.body, &need.body) {
            (None, None) => true,
            (Some(s), Some(n)) => s == OPAQUE || s == n,
            _ => false,
        };
        let answer = match (&self.answer, &need.answer) {
            (_, Some(n)) if n.ty == OPAQUE && self.answer.as_ref().is_none_or(|s| s.stream == n.stream) => true,
            (None, None) => true,
            (Some(s), Some(n)) => s.ty == n.ty && s.stream == n.stream,
            _ => false,
        };
        self.verb == need.verb && body && answer
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.verb)?;
        if let Some(b) = &self.body {
            write!(f, " {b}")?;
        }
        if let Some(a) = &self.answer {
            write!(f, " -> {}{}", a.ty, if a.stream { "*" } else { "" })?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Parsed

/// A parsed header file.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub imports: Vec<Import>,
    pub messages: Vec<MessageDecl>,
    pub objects: Vec<ObjectDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageDecl {
    pub name: Ident,
    pub doc: String,
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: Ident,
    pub ty: Ident,
    pub doc: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectDecl {
    pub name: Ident,
    pub doc: String,
    /// `guide "<file>"`, relative to the header.
    pub guide: Option<(String, Pos)>,
    pub needs: Vec<NeedDecl>,
    pub serves: Vec<EntryDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeedDecl {
    pub at: image::Path,
    pub doc: String,
    pub entries: Vec<EntryDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryDecl {
    pub path: image::Path,
    pub doc: String,
    pub requests: Vec<RequestDecl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestDecl {
    pub verb: Verb,
    pub pos: Pos,
    pub body: Option<Ident>,
    /// The answer's type, and whether it is a stream.
    pub answer: Option<(Ident, bool)>,
}

pub fn parse(src: &str) -> Result<File> {
    Parser::new(src, true)?.header()
}

impl Parser {
    /// Consecutive `##` lines, joined; empty if there are none.
    fn docs(&mut self) -> String {
        let mut lines = vec![];
        while let Tok::Doc(line) = self.peek() {
            lines.push(line.clone());
            self.next();
        }
        lines.join("\n")
    }

    fn no_docs(&mut self, doc: &str, pos: Pos) -> Result<()> {
        if !doc.is_empty() {
            bail!("{pos}: a `##` comment documents what follows it, and nothing documentable does");
        }
        Ok(())
    }

    fn header(&mut self) -> Result<File> {
        let mut file = File { imports: vec![], messages: vec![], objects: vec![] };
        loop {
            let doc = self.docs();
            let pos = self.pos();
            match self.peek().clone() {
                Tok::Eof => {
                    self.no_docs(&doc, pos)?;
                    return Ok(file);
                }
                Tok::Ident(kw) if kw == "import" => {
                    self.no_docs(&doc, pos)?;
                    file.imports.push(self.import()?);
                }
                Tok::Ident(kw) if kw == "message" => file.messages.push(self.message(doc)?),
                Tok::Ident(kw) if kw == "object" => file.objects.push(self.object(doc)?),
                other => bail!("{pos}: expected `import`, `message` or `object`, found {other}"),
            }
        }
    }

    fn message(&mut self, doc: String) -> Result<MessageDecl> {
        self.expect_kw("message")?;
        let name = self.ident("message name", false)?;
        self.expect(Tok::LBrace)?;
        let mut fields = vec![];
        loop {
            let field_doc = self.docs();
            if *self.peek() == Tok::RBrace {
                let pos = self.pos();
                self.no_docs(&field_doc, pos)?;
                self.next();
                return Ok(MessageDecl { name, doc, fields });
            }
            let fname = self.ident("field name", false)?;
            self.expect(Tok::Colon)?;
            let ty = self.ident("field type", false)?;
            fields.push(FieldDecl { name: fname, ty, doc: field_doc });
            if *self.peek() == Tok::Comma {
                self.next();
            }
        }
    }

    fn object(&mut self, doc: String) -> Result<ObjectDecl> {
        self.expect_kw("object")?;
        let name = self.ident("object name", true)?;
        self.expect(Tok::LBrace)?;
        let mut obj = ObjectDecl { name, doc, guide: None, needs: vec![], serves: vec![] };
        loop {
            let doc = self.docs();
            let (tok, pos) = self.next();
            match tok {
                Tok::RBrace => {
                    self.no_docs(&doc, pos)?;
                    return Ok(obj);
                }
                Tok::Ident(kw) if kw == "needs" => {
                    let at = self.path()?;
                    let entries = self.entries()?;
                    obj.needs.push(NeedDecl { at, doc, entries });
                }
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
            let mut requests = vec![self.request()?];
            while *self.peek() == Tok::Comma {
                self.next();
                requests.push(self.request()?);
            }
            entries.push(EntryDecl { path, doc, requests });
        }
    }

    fn request(&mut self) -> Result<RequestDecl> {
        let (tok, pos) = self.next();
        let verb = match &tok {
            Tok::Ident(w) => Verb::from_keyword(w),
            _ => None,
        };
        let Some(verb) = verb else {
            let verbs: Vec<&str> = crate::ns::VERBS.iter().map(|v| v.keyword()).collect();
            bail!("{pos}: expected a verb ({}), found {tok}", verbs.join(", "));
        };
        let body = match self.peek() {
            Tok::Ident(w) if Verb::from_keyword(w).is_none() => Some(self.ident("body type", false)?),
            _ => None,
        };
        let answer = if *self.peek() == Tok::Arrow {
            self.next();
            let ty = self.ident("answer type", false)?;
            let stream = *self.peek() == Tok::Star;
            if stream {
                self.next();
            }
            Some((ty, stream))
        } else {
            None
        };
        Ok(RequestDecl { verb, pos, body, answer })
    }
}

// ---------------------------------------------------------------------------
// Resolving

fn err_at(id: &Ident, msg: impl fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{}: {msg}", id.pos)
}

/// Resolves `file`. `import` provides the header behind each import's
/// source, and `guide` reads a guide's file, named relative to the header.
pub fn resolve(
    file: &File,
    import: &mut dyn FnMut(&Source) -> Result<Header>,
    guide: &dyn Fn(&str) -> Result<String>,
) -> Result<Header> {
    let mut messages: BTreeMap<String, MessageType> = BTreeMap::new();
    let mut from: BTreeMap<String, String> = BTreeMap::new();
    for imp in &file.imports {
        let header = import(&imp.from).with_context(|| format!("{}: importing from {}", imp.pos, imp.from))?;
        for name in &imp.names {
            let Some(ty) = header.messages.get(&name.name) else {
                bail!(err_at(name, format!("{} has no message type `{}`", imp.from, name.name)));
            };
            reserve(name)?;
            if let Some(prev) = from.insert(name.name.clone(), imp.from.to_string()) {
                bail!(err_at(name, format!("`{}` is already imported from {prev}", name.name)));
            }
            messages.insert(name.name.clone(), ty.clone());
        }
    }
    for m in &file.messages {
        reserve(&m.name)?;
        if let Some(prev) = from.get(&m.name.name) {
            bail!(err_at(&m.name, format!("`{}` is imported from {prev}, and declared here too", m.name.name)));
        }
        if messages.contains_key(&m.name.name) {
            bail!(err_at(&m.name, format!("message `{}` is declared twice", m.name.name)));
        }
        let mut fields: Vec<(String, FieldType)> = vec![];
        for f in &m.fields {
            if fields.iter().any(|(n, _)| *n == f.name.name) {
                bail!(err_at(&f.name, format!("`{}` has two fields called `{}`", m.name.name, f.name.name)));
            }
            let Some(ty) = FieldType::from_name(&f.ty.name).filter(|_| !["ArrayBuffer", "boolean"].contains(&f.ty.name.as_str())) else {
                bail!(err_at(
                    &f.ty,
                    format!(
                        "`{}` is not a field type; fields are string, bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64 or bytes",
                        f.ty.name
                    )
                ));
            };
            fields.push((f.name.name.clone(), ty));
        }
        let field_docs = m.fields.iter().map(|f| f.doc.clone()).collect();
        messages.insert(m.name.name.clone(), MessageType { name: m.name.name.clone(), fields, doc: m.doc.clone(), field_docs });
    }

    let mut objects: Vec<Object> = vec![];
    for o in &file.objects {
        if objects.iter().any(|prev| prev.name == o.name.name) {
            bail!(err_at(&o.name, format!("object `{}` is declared twice", o.name.name)));
        }
        let guide = match &o.guide {
            Some((path, pos)) => Some(guide(path).with_context(|| format!("{pos}: reading the guide {path:?}"))?),
            None => None,
        };
        let mut needs: Vec<Need> = vec![];
        for n in &o.needs {
            let at = pattern(&n.at)?;
            if let Some(prev) = needs.iter().find(|p| same(&p.at, &at)) {
                bail!("{}: `needs {}` overlaps `needs {}` exactly; one block per mount point", n.at.pos, at, prev.at);
            }
            needs.push(Need { at, doc: n.doc.clone(), entries: entries(&n.entries, &messages)? });
        }
        let serves = entries(&o.serves, &messages)?;
        objects.push(Object { name: o.name.name.clone(), doc: o.doc.clone(), guide, needs, serves });
    }
    Ok(Header { messages, objects })
}

fn reserve(name: &Ident) -> Result<()> {
    if name.name == OPAQUE {
        bail!(err_at(name, format!("`{OPAQUE}` is reserved: it stands for any message")));
    }
    Ok(())
}

fn pattern(p: &image::Path) -> Result<Pattern> {
    Pattern::parse(&p.path).map_err(|e| anyhow::anyhow!("{}: {e}", p.pos))
}

/// Two patterns that match exactly the same paths.
fn same(a: &Pattern, b: &Pattern) -> bool {
    a.specificity(b) == Ordering::Equal && a.covers(b) && b.covers(a)
}

fn entries(decls: &[EntryDecl], messages: &BTreeMap<String, MessageType>) -> Result<Vec<Entry>> {
    let ty = |id: &Ident| -> Result<String> {
        if id.name != OPAQUE && !messages.contains_key(&id.name) {
            bail!(err_at(id, format!("unknown message type `{}`; is it imported?", id.name)));
        }
        Ok(id.name.clone())
    };
    let mut out: Vec<Entry> = vec![];
    for e in decls {
        let path = pattern(&e.path)?;
        if let Some(prev) = out.iter().find(|p| same(&p.path, &path)) {
            bail!("{}: `{path}` is already listed, as `{}`", e.path.pos, prev.path);
        }
        let mut requests: Vec<Request> = vec![];
        for r in &e.requests {
            let body = r.body.as_ref().map(ty).transpose()?;
            let answer = match &r.answer {
                Some((id, stream)) => Some(Answer { ty: ty(id)?, stream: *stream }),
                None => None,
            };
            let req = Request { verb: r.verb, body, answer };
            if requests.iter().any(|prev| prev.verb == req.verb && prev.body == req.body) {
                bail!("{}: `{path}` takes `{req}` twice", r.pos);
            }
            requests.push(req);
        }
        out.push(Entry { path, doc: e.doc.clone(), requests });
    }
    Ok(out)
}

/// Reads and resolves the header at `path`. `from env` is the runtime's
/// header, `from Name` is `Name.iface` next to this one, and a quoted path
/// is relative to this one.
pub fn load(path: &Path) -> Result<Header> {
    load_depth(path, 0)
}

fn load_depth(path: &Path, depth: usize) -> Result<Header> {
    if depth > 16 {
        bail!("headers import each other in a loop");
    }
    let src = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file = parse(&src).with_context(|| format!("parsing {}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut import = |s: &Source| -> Result<Header> {
        match s {
            Source::Module(m) if m.name == "env" => Ok(env().clone()),
            Source::Module(m) => load_depth(&dir.join(format!("{}.iface", m.name)), depth + 1),
            Source::Path(p) => load_depth(&dir.join(p), depth + 1),
        }
    };
    let guide = |p: &str| -> Result<String> { Ok(std::fs::read_to_string(dir.join(p))?) };
    resolve(&file, &mut import, &guide).with_context(|| format!("in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREETER: &str = r#"
        import { Line } from env

        ## Configures a Greeter.
        message Greeting {
          ## Put before each name.
          prefix: string
          ## End with `!` rather than `.`.
          excited: bool
        }

        message Stats { lines: i32, chars: i64, average: f64 }

        ## Greets every name typed on the console.
        ##
        ## And reports statistics.
        object Greeter {
          ## The console, for names and greetings.
          needs /dev/cons {
            /  watch -> Line*, write Opaque
          }
          needs /shouter { / write Line }

          serves {
            ## Configures the greeting.
            /greeting  write Greeting
            /stats     read -> Stats
          }
        }
    "#;

    fn resolve_str(src: &str) -> Result<Header> {
        let file = parse(src)?;
        let mut import = |s: &Source| -> Result<Header> {
            match s {
                Source::Module(m) if m.name == "env" => Ok(env().clone()),
                other => bail!("no {other} here"),
            }
        };
        resolve(&file, &mut import, &|p| Ok(format!("the guide in {p}")))
    }

    fn err(src: &str) -> String {
        format!("{:#}", resolve_str(src).unwrap_err())
    }

    #[test]
    fn parses_and_resolves_a_header() {
        let h = resolve_str(GREETER).unwrap();
        assert_eq!(h.messages.keys().collect::<Vec<_>>(), ["Greeting", "Line", "Stats"], "its own types and its imports");
        let g = &h.messages["Greeting"];
        assert_eq!(g.doc, "Configures a Greeter.");
        assert_eq!(g.fields, [("prefix".into(), FieldType::String), ("excited".into(), FieldType::Bool)]);
        assert_eq!(g.field_docs, ["Put before each name.", "End with `!` rather than `.`."]);

        let o = h.object("Greeter").unwrap();
        assert_eq!(o.doc, "Greets every name typed on the console.\n\nAnd reports statistics.");
        assert_eq!(o.needs.len(), 2);
        assert_eq!((o.needs[0].at.as_str(), o.needs[0].doc.as_str()), ("/dev/cons", "The console, for names and greetings."));
        let reqs: Vec<String> = o.needs[0].entries[0].requests.iter().map(|r| r.to_string()).collect();
        assert_eq!(reqs, ["watch -> Line*", "write Opaque"]);
        assert_eq!(o.serves[0].doc, "Configures the greeting.");
        assert_eq!(o.serves[1].requests[0].to_string(), "read -> Stats");
        assert_eq!(o.guide, None);
    }

    #[test]
    fn guides_are_read_in() {
        let h = resolve_str(r#"object A { guide "./A.md" }"#).unwrap();
        assert_eq!(h.objects[0].guide.as_deref(), Some("the guide in ./A.md"));
    }

    #[test]
    fn env_declares_the_built_in_objects() {
        let env = env();
        for name in ["Environment.Console", "Environment.Tcp", "Environment.Files", "Packed.Files"] {
            assert!(env.object(name).is_some(), "{name}");
        }
        assert!(env.messages.contains_key("Line"));
        // Until as/env.ts is generated from env.iface, the two must agree.
        let ts = crate::messages::parse(include_str!("../as/env.ts")).unwrap();
        let shapes = |m: &BTreeMap<String, MessageType>| m.values().map(|t| (t.name.clone(), t.fields.clone())).collect::<Vec<_>>();
        assert_eq!(shapes(&env.messages), shapes(&ts));
    }

    #[test]
    fn rejects_mistakes() {
        assert!(err("message A { x: Strin }").contains("`Strin` is not a field type"));
        assert!(err("message A { x: ArrayBuffer }").contains("not a field type"), "headers write `bytes`");
        assert!(err("message A { x: i32 x: i32 }").contains("two fields called `x`"));
        assert!(err("message A {} message A {}").contains("declared twice"));
        assert!(err("message Opaque {}").contains("reserved"));
        assert!(err("import { Line } from env message Line { text: string }").contains("imported from env, and declared here too"));
        assert!(err("import { Nope } from env").contains("env has no message type `Nope`"));
        assert!(err("import { X } from Other").contains("no Other here"));
        assert!(err("object A { serves { /x read -> Nope } }").contains("unknown message type `Nope`"));
        assert!(err("object A { serves { /x read, read } }").contains("takes `read` twice"));
        assert!(err("object A { serves { /{a} read /{b} stat } }").contains("already listed"));
        assert!(err("object A { needs /** { / read } needs /** { / stat } }").contains("one block per mount point"));
        assert!(err("object A { serves { /a/**/b read } }").contains("`**` can only come last"));
        assert!(err("object A { serves { /x frob } }").contains("expected a verb"));
        assert!(err("object A { serves { /x } }").contains("expected a verb"));
        assert!(err("## nothing\n").contains("documents what follows"));
        assert!(err("## an import\nimport { Line } from env").contains("documents what follows"));
        assert!(err("object A { guide \"a\" guide \"b\" }").contains("names its guide twice"));
    }

    #[test]
    fn requests_take_bodies_by_type() {
        let h = resolve_str("message T {} object A { serves { /h control Text, control, read -> Opaque*, write Opaque } }")
            .err()
            .map(|e| e.to_string());
        assert!(h.unwrap().contains("unknown message type `Text`"));
        let h = resolve_str(
            "import { Text } from env message T {} object A { serves { /h control Text, control, read -> Opaque*, write Opaque } }",
        )
        .unwrap();
        let e = &h.objects[0].serves[0];
        assert_eq!(e.takes(Verb::Control, Some("Text")).unwrap().body.as_deref(), Some("Text"));
        assert_eq!(e.takes(Verb::Control, None).unwrap().body, None);
        assert_eq!(e.takes(Verb::Control, Some("T")), Err(Status::Invalid));
        assert_eq!(e.takes(Verb::Write, Some("T")).unwrap().body.as_deref(), Some(OPAQUE));
        assert_eq!(e.takes(Verb::Write, None), Err(Status::Invalid));
        assert_eq!(e.takes(Verb::Stat, None), Err(Status::NotSupported));
    }

    #[test]
    fn opaque_meets_needs_in_one_direction() {
        let r = |src: &str| {
            let h = resolve_str(&format!("import {{ Line, Data }} from env object A {{ serves {{ / {src} }} }}")).unwrap();
            h.objects[0].serves[0].requests[0].clone()
        };
        assert!(r("write Opaque").meets(&r("write Line")), "a server taking anything takes a Line");
        assert!(!r("write Line").meets(&r("write Opaque")), "but a need to send anything isn't met by one taking Lines");
        assert!(r("read -> Data").meets(&r("read -> Opaque")));
        assert!(r("remove").meets(&r("remove -> Opaque")), "no answer is fine where any is");
        assert!(!r("read -> Opaque").meets(&r("read -> Data")));
        assert!(!r("watch -> Line").meets(&r("watch -> Line*")), "a stream isn't a single answer");
        assert!(!r("watch -> Line*").meets(&r("watch -> Opaque")));
        assert!(r("watch -> Line*").meets(&r("watch -> Opaque*")));
        assert!(!r("write Line").meets(&r("write")));
        assert!(!r("read").meets(&r("stat")));
    }

    #[test]
    fn picks_the_most_specific_entry_and_need() {
        let h = resolve_str(
            "import { Data, Text } from env
            object A {
              needs /net/tcp { /{conn} write Data }
              needs /** { /** read -> Opaque }
              serves { /hits read -> Text  /** read -> Data }
            }",
        )
        .unwrap();
        let o = &h.objects[0];
        assert_eq!(o.serving("/hits").unwrap().path.as_str(), "/hits");
        assert_eq!(o.serving("/").unwrap().path.as_str(), "/**");
        assert_eq!(o.need("/net/tcp").unwrap().at.as_str(), "/net/tcp");
        assert_eq!(o.need("/www").unwrap().at.as_str(), "/**");
        assert!(o.need("/net/tcp").unwrap().entry("/").is_none(), "`{{conn}}` is never empty");
    }

    #[test]
    fn headers_round_trip_through_bson() {
        let h = resolve_str(GREETER).unwrap();
        let doc = bson::serialize_to_document(&h).unwrap();
        let back: Header = bson::deserialize_from_document(doc).unwrap();
        assert_eq!(back, h);
    }
}
