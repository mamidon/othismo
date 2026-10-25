//! Headers, resolved: what each object serves and needs, and the protocols
//! and traits they are made of, every name in them declared. A resolved
//! header stands on its own, so it can travel inside a module and inside a
//! built image; the compiler resolves them from `.oth` files.

use protocol::{status::Status, verb::Verb};
use protocol::path::{MountPoint, PathName};
use protocol::pattern::PathPattern;
use protocol::content_type::ContentType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// Every protocol the header can name, its own and its imports.
    #[serde(default)]
    pub protocols: BTreeMap<String, Protocol>,
    /// Every trait the header can name, its own and its imports.
    #[serde(default)]
    pub traits: BTreeMap<String, Trait>,
    /// The objects it declares itself.
    pub objects: Vec<Object>,
    /// The objects it imports, by the names it gives them.
    #[serde(default)]
    pub imported: BTreeMap<String, Object>,
    /// The names it exports: its own declarations written `export`, and
    /// imported ones it names in `export { ... }`.
    #[serde(default)]
    pub exports: Vec<String>,
    /// An image's top-level needs: what it needs from whoever instantiates it.
    #[serde(default)]
    pub needs: Vec<Need>,
    /// Every message type the header can name, its schemas blocks' and its
    /// imports', with the content type of the format that declares it.
    #[serde(default)]
    pub types: BTreeMap<String, String>,
}

/// `protocol <Name> { ... }`: the requests one path takes, with any
/// protocols it names written out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Protocol {
    pub name: String,
    pub doc: String,
    pub requests: Vec<Request>,
}

/// `trait <Name> { ... }`: the shape of a tree, with any protocols and
/// traits it names written out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trait {
    pub name: String,
    pub doc: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub name: String,
    /// The object's `##` comment, Markdown.
    pub doc: String,
    /// The traits it declares, whose entries are among those it serves.
    #[serde(default)]
    pub traits: Vec<String>,
    /// The guide's Markdown, if the object names one.
    pub guide: Option<String>,
    pub needs: Vec<Need>,
    pub serves: Vec<Entry>,
    /// The file it was read from: a `.oth` file, whose module is the `.wasm`
    /// beside it, or a module. `None` for the runtime's own.
    #[serde(skip)]
    pub origin: Option<PathBuf>,
}

/// `needs <mount point pattern> { entries }`, or `needs <mount point> <Trait>`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Need {
    pub at: PathPattern,
    pub doc: String,
    pub entries: Vec<Entry>,
    /// The trait it names, if it is a trait's whole tree.
    #[serde(default)]
    pub shape: Option<String>,
}

/// A path pattern and the requests it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathPattern,
    pub doc: String,
    pub requests: Vec<Request>,
}

/// `[operator] <verb> [content type] [-> content type[*]]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Only whoever runs the image may make it.
    #[serde(default)]
    pub operator: bool,
    pub verb: Verb,
    /// The content type of the payload it takes, if it takes one.
    pub body: Option<ContentType>,
    /// The message type it names for that payload, if it names one: its
    /// format's to read. The runtime goes by the content type alone.
    #[serde(default)]
    pub message: Option<String>,
    /// `None`: answered with a status alone. `stat` and `list` always
    /// answer stat records, so they are written bare.
    pub answer: Option<Answer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    /// The answer's content type.
    pub ty: ContentType,
    /// The message type it names, if it names one.
    #[serde(default)]
    pub message: Option<String>,
    /// `type*`: any number of answers, the last one marked done.
    pub stream: bool,
}

impl Header {
    /// The object it declares itself as `name`.
    pub fn object(&self, name: &str) -> Option<&Object> {
        self.objects.iter().find(|o| o.name == name)
    }

    /// The object `name` stands for in it: its own, or one it imports.
    pub fn object_named(&self, name: &str) -> Option<&Object> {
        self.object(name).or_else(|| self.imported.get(name))
    }
}

impl Trait {
    /// An object that serves the trait's tree and needs nothing: how the
    /// runtime sees an instance declared by the trait.
    pub fn as_object(&self) -> Object {
        Object {
            name: self.name.clone(),
            doc: self.doc.clone(),
            traits: vec![],
            guide: None,
            needs: vec![],
            serves: self.entries.clone(),
            origin: None,
        }
    }
}

impl Object {
    /// The entry serving `path`, a path in the object's served tree.
    pub fn serving(&self, path: &PathName) -> Option<&Entry> {
        PathPattern::best(&self.serves, |e| &e.path, path)
    }

    /// Whether `other` has the same shape: what it serves, needs and declares.
    pub fn same_shape(&self, other: &Object) -> bool {
        self.name == other.name && self.traits == other.traits && self.needs == other.needs && self.serves == other.serves
    }

    /// The `needs` block for the mount point `at`.
    pub fn need(&self, at: &MountPoint) -> Option<&Need> {
        PathPattern::best(&self.needs, |n| &n.at, at.path())
    }
}

impl Need {
    pub fn entry(&self, path: &PathName) -> Option<&Entry> {
        PathPattern::best(&self.entries, |e| &e.path, path)
    }
}

impl Entry {
    /// The request a `verb` with a payload of content type `body` fits, as
    /// the checker sees it: `NotSupported` if the entry doesn't take the
    /// verb, and `Invalid` if it does but not with that payload.
    pub fn takes(&self, verb: Verb, body: Option<&ContentType>) -> Result<&Request, Status> {
        let mut with_verb = self.requests.iter().filter(|r| r.verb == verb).peekable();
        if with_verb.peek().is_none() {
            return Err(Status::NOT_SUPPORTED);
        }
        with_verb.find(|r| body_fits(r.body.as_ref(), body)).ok_or(Status::INVALID)
    }

    /// Whether the entry takes `verb` from a sender that is the operator or
    /// not: all the runtime asks, since it never looks at payloads.
    /// `Forbidden` if only the operator may make it.
    pub fn takes_verb(&self, verb: Verb, operator: bool) -> Result<(), Status> {
        let mut with_verb = self.requests.iter().filter(|r| r.verb == verb).peekable();
        if with_verb.peek().is_none() {
            return Err(Status::NOT_SUPPORTED);
        }
        if operator || with_verb.any(|r| !r.operator) { Ok(()) } else { Err(Status::FORBIDDEN) }
    }

    /// Whether `verb` may be answered more than once here.
    pub fn streams(&self, verb: Verb) -> bool {
        self.requests.iter().any(|r| r.verb == verb && r.answer.as_ref().is_some_and(|a| a.stream))
    }
}

/// Whether a payload of content type `given` fits where `want` is declared.
pub fn body_fits(want: Option<&ContentType>, given: Option<&ContentType>) -> bool {
    match (want, given) {
        (None, None) => true,
        (Some(w), Some(g)) => g.fits(w),
        _ => false,
    }
}

impl Request {
    /// Whether this request, as a server serves it, meets `need`: the same
    /// verb, a payload the server takes, and an answer the need takes.
    /// Wildcards go one way: a server taking `*/*` takes any payload, and a
    /// need for a `*/*` answer accepts any answer, or none; but a server
    /// that only promises `*/*` doesn't meet a need for `text/plain`.
    ///
    /// Where both name a message type, it must be the same one. A server
    /// that names one takes only that, so a need must name it too; a need
    /// that names one is met by a server that takes any payload of its
    /// content type.
    pub fn meets(&self, need: &Request) -> bool {
        let answer = match (&self.answer, &need.answer) {
            (_, Some(n)) if n.ty == ContentType::ANY && self.answer.as_ref().is_none_or(|s| s.stream == n.stream) => true,
            (None, None) => true,
            (Some(s), Some(n)) => {
                s.ty.fits(&n.ty) && s.stream == n.stream && (n.message.is_none() || n.message == s.message)
            }
            _ => false,
        };
        let operator = !self.operator || need.operator;
        let message = self.message.is_none() || self.message == need.message;
        self.verb == need.verb && operator && body_fits(self.body.as_ref(), need.body.as_ref()) && message && answer
    }

    /// Whether the verb's answer is set by the protocol: `stat` and `list`
    /// answer stat records, and are written bare.
    pub fn implied(verb: Verb) -> Option<Answer> {
        match verb {
            Verb::Stat => Some(Answer { ty: ContentType::STAT, message: None, stream: false }),
            Verb::List => Some(Answer { ty: ContentType::STAT, message: None, stream: true }),
            _ => None,
        }
    }
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let reqs: Vec<String> = self.requests.iter().map(|r| r.to_string()).collect();
        write!(f, "{} {}", self.path, reqs.join(", "))
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.operator {
            f.write_str("operator ")?;
        }
        write!(f, "{}", self.verb)?;
        if let Some(b) = &self.body {
            write!(f, " {}", self.message.as_deref().unwrap_or(b.as_str()))?;
        }
        if let Some(a) = &self.answer
            && Request::implied(self.verb).is_none()
        {
            let ty = a.message.as_deref().unwrap_or(if a.ty == ContentType::STAT { ContentType::STAT_NAME } else { a.ty.as_str() });
            write!(f, " -> {ty}{}", if a.stream { "*" } else { "" })?;
        }
        Ok(())
    }
}

