//! Completion for the command line: what can come next where the cursor
//! is, and why, from what the shell knows of the running image.
//!
//! In turn: the verb; the path, from the view looked from, what its
//! servers have listed, exposed and declared in their headers; the payload's
//! type, from the server's header; and a message's fields and their values,
//! from its format. Whatever the runtime would refuse is still offered,
//! marked unusable, with the reason, so the operator can see why.

use crate::command::HELP;
use crate::session::Session;
use crate::world::{Viewpoint, World};
use format::{AllSchemas, Formats, MessageDoc};
use protocol::ns::{Status, VERBS, Verb};
use protocol::payload::Stat;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Completion {
    /// Where in the line what is chosen goes, up to the cursor.
    pub from: usize,
    pub items: Vec<Item>,
    /// What to say when there is nothing to choose.
    pub hint: Option<String>,
    /// A served directory that would be worth listing: an instance, and
    /// the path in what it serves.
    pub wants: Option<(usize, String)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub label: String,
    /// What goes in its place; the cursor goes to `caret`, or the end.
    pub insert: String,
    pub caret: Option<usize>,
    /// A few words beside it.
    pub detail: String,
    /// What it is: its signature, then its documentation.
    pub signature: String,
    pub doc: String,
    /// Where the documentation comes from.
    pub source: String,
    /// Whether the runtime would let it through.
    pub usable: bool,
    /// Whether to complete again once it is chosen, as after a directory.
    pub more: bool,
    /// Stands for a name the operator has to type, such as `{name}`.
    pub placeholder: bool,
}

impl Item {
    fn new(label: impl Into<String>, insert: &str) -> Item {
        let (insert, caret) = match insert.split_once('|') {
            Some((a, b)) => (format!("{a}{b}"), Some(a.len())),
            None => (insert.to_string(), None),
        };
        Item { label: label.into(), insert, caret, usable: true, ..Item::default() }
    }

    fn detail(mut self, s: impl Into<String>) -> Item {
        self.detail = s.into();
        self
    }

    fn doc(mut self, signature: impl Into<String>, doc: impl Into<String>, source: impl Into<String>) -> Item {
        (self.signature, self.doc, self.source) = (signature.into(), doc.into(), source.into());
        self
    }
}

/// What a verb is for, from the protocol.
pub fn verb_doc(v: Verb) -> &'static str {
    match v {
        Verb::Read => "All of what is at the path, once, with its content type.",
        Verb::Stat => "What is at the path: one stat record.",
        Verb::List => "What is directly under the path: a stat record each, as a stream.",
        Verb::Watch => "Payloads as they happen, until the server ends the stream or you cancel it.",
        Verb::Write => "Change what is at the path, or act on it. The server may answer with a payload.",
        Verb::Create => "Make something new under the path; conventionally answered with a stat record.",
        Verb::Remove => "Remove what is at the path.",
    }
}

/// Completes `before`, the line up to the cursor. `listed` says what has
/// been listed under a path an instance serves, if it has been.
pub fn complete(s: &Session, listed: &dyn Fn(usize, &str) -> Option<Vec<Stat>>, before: &str) -> Completion {
    let w = &s.world;
    let lead = before.len() - before.trim_start().len();
    let line = &before[lead..];
    if let Some(rest) = line.strip_prefix(':') {
        return shell_command(s, rest, lead + 1);
    }
    let Some(here) = s.here else {
        return hint("Look from somewhere first: requests are made from a point of view. Esc, then pick a root, or :look <root>.");
    };
    let mut words = line.splitn(3, char::is_whitespace);
    let (verb, path, rest) = (words.next().unwrap_or(""), words.next(), words.next());
    // The verb
    let Some(path) = path else {
        let items = VERBS
            .iter()
            .filter(|v| v.keyword().starts_with(verb))
            .map(|&v| {
                Item::new(v.keyword(), &format!("{} ", v.keyword()))
                    .detail(if v.is_safe() { "safe" } else { "changes things" })
                    .doc(v.keyword(), verb_doc(v), "the protocol")
            })
            .collect();
        return Completion { from: lead, items, ..Completion::default() };
    };
    let Some(v) = Verb::from_keyword(verb) else {
        return hint(&format!("`{verb}` isn't a verb; the verbs are {}.", VERBS.map(|v| v.keyword()).join(", ")));
    };
    // The path
    let Some(rest) = rest else {
        let from = before.len() - path.len();
        return Completion { from, ..paths(w, here, v, path, listed) };
    };
    payload(s, here, v, path, rest, before.len())
}

fn hint(text: &str) -> Completion {
    Completion { hint: Some(text.to_string()), ..Completion::default() }
}

fn shell_command(s: &Session, rest: &str, at: usize) -> Completion {
    let w = &s.world;
    if let Some(arg) = rest.strip_prefix("look").and_then(|a| a.strip_prefix(char::is_whitespace)) {
        let arg = arg.trim_start();
        let from = at + rest.len() - arg.len();
        let mut items: Vec<Item> = std::iter::once(Viewpoint::Root)
            .chain((0..w.instances().len()).map(Viewpoint::Instance))
            .filter(|&v| w.has_view(v) && w.name(v).starts_with(arg))
            .map(|v| {
                let root = w.roots().contains(&v);
                let what = match v {
                    Viewpoint::Root => "the image's own view".to_string(),
                    Viewpoint::Instance(i) => format!("{} · {}", w.instance(i).type_name, w.class(i).name()),
                };
                Item::new(w.name(v), w.name(v)).detail(if root { format!("{what} · a root") } else { what })
            })
            .collect();
        items.sort_by_key(|i| !i.detail.ends_with("a root"));
        return Completion { from, items, ..Completion::default() };
    }
    if rest.contains(char::is_whitespace) {
        return Completion::default();
    }
    let items = HELP
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix(':'))
        .filter_map(|l| {
            let name = l.split_whitespace().next()?;
            name.starts_with(rest).then(|| {
                let (usage, doc) = l.split_once("  ").unwrap_or((l, ""));
                Item::new(format!(":{name}"), &format!("{name} ")).doc(format!(":{}", usage.trim()), doc.trim(), "the shell")
            })
        })
        .collect();
    Completion { from: at, items, ..Completion::default() }
}

/// Paths in `here`'s view for `verb`, under the directory `partial` names.
fn paths(w: &World, here: Viewpoint, verb: Verb, partial: &str, listed: &dyn Fn(usize, &str) -> Option<Vec<Stat>>) -> Completion {
    let p = if partial.starts_with('/') { partial.to_string() } else { format!("/{partial}") };
    let (dir, stem) = p.split_at(p.rfind('/').expect("starts with /") + 1);
    let mut items: Vec<Item> = vec![];
    // Mount points, which lead on to what is served there, and the
    // directories on the way to them.
    for (name, mount) in w.mounts_under(here, dir) {
        let mut item = path_item(w, here, verb, &format!("{dir}{name}"), true, &name);
        if !mount {
            (item.detail, item.usable) = ("mount points".into(), true);
        }
        items.push(item);
    }
    // What is served there.
    let mut wants = None;
    if let Ok(r) = w.resolve(here, dir) {
        let listing = listed(r.inst, &r.path);
        if listing.is_none() {
            wants = Some((r.inst, r.path.clone()));
        }
        let mut names: Vec<(String, bool)> = listing.unwrap_or_default().into_iter().map(|s| (s.name, s.dir)).collect();
        let (known, patterns) = w.known_under(r.inst, &r.path);
        for k in known {
            match names.iter_mut().find(|(n, _)| *n == k.name) {
                Some((_, d)) => *d |= k.dir,
                None => names.push((k.name, k.dir)),
            }
        }
        for (name, is_dir) in names {
            if !items.iter().any(|i| i.label.trim_end_matches('/') == name) {
                items.push(path_item(w, here, verb, &format!("{dir}{name}"), is_dir, &name));
            }
        }
        for pat in patterns {
            let usable = w.check(here, verb, &format!("{dir}_")).is_ok();
            let mut item = Item::new(pat.segment.clone(), "").doc(format!("{dir}{}", pat.segment), pat.doc, source(w, r.inst));
            item.detail = if usable { "type a name".into() } else { why(w, here, verb, &format!("{dir}_")) };
            item.usable = usable;
            item.placeholder = true;
            items.push(item);
        }
    }
    items.retain(|i| i.placeholder || i.label.starts_with(stem));
    items.sort_by_key(|i| (!i.usable, !i.more, i.placeholder, i.label.clone()));
    let hint = items.is_empty().then(|| format!("Nothing under {dir} starts with `{stem}`, as far as is known."));
    Completion { from: 0, items, hint, wants }
}

/// A path as a choice: usable as it is, a directory to go on into, or
/// refused, with why.
fn path_item(w: &World, here: Viewpoint, verb: Verb, path: &str, is_dir: bool, name: &str) -> Item {
    let checked = w.check(here, verb, path);
    let r = w.resolve(here, path).ok();
    let owner = r.as_ref().map(|r| w.instance(r.inst).name.clone()).unwrap_or_default();
    let entry = r.as_ref().and_then(|r| w.instance(r.inst).interface.serving(&r.path).cloned());
    let (signature, doc) = match &entry {
        Some(e) => (e.to_string(), e.doc.clone()),
        None => (path.to_string(), r.as_ref().map(|r| w.instance(r.inst).interface.doc.clone()).unwrap_or_default()),
    };
    let src = r.as_ref().map(|r| source(w, r.inst)).unwrap_or_default();
    let item = match (&checked, is_dir) {
        (Ok(_), _) => Item::new(name, &format!("{path} ")).detail(owner),
        (Err(_), true) => {
            // Going on into it is what matters; say why it won't do itself
            // only if that isn't simply that nothing is served there.
            let why = why(w, here, verb, path);
            let detail = if why == "not_found" { owner } else { format!("{owner} · itself: {why}") };
            let mut i = Item::new(format!("{name}/"), &format!("{path}/")).detail(detail);
            i.more = true;
            i
        }
        (Err(_), false) => {
            let mut i = Item::new(name, &format!("{path} ")).detail(format!("{owner} · {}", why(w, here, verb, path)));
            i.usable = false;
            i
        }
    };
    item.doc(signature, doc, src)
}

/// Why the runtime would refuse `verb path` from `here`, in a few words.
fn why(w: &World, here: Viewpoint, verb: Verb, path: &str) -> String {
    match w.check(here, verb, path) {
        Ok(_) => String::new(),
        Err(Status::ReadOnly) => "read-only from here".into(),
        Err(Status::Forbidden) => format!("operator only; {} isn't one", w.name(here)),
        Err(Status::NotSupported) => format!("doesn't take {}", verb.keyword()),
        Err(Status::NotFound) if !w.is_operator(here) => format!("not among {}'s needs", w.name(here)),
        Err(s) => s.name().to_string(),
    }
}

fn source(w: &World, inst: usize) -> String {
    format!("{}'s header", w.instance(inst).interface.name)
}

/// After the path: the payload's type, then a message's fields and values.
fn payload(s: &Session, here: Viewpoint, verb: Verb, path: &str, rest: &str, end: usize) -> Completion {
    let w = &s.world;
    let r = match w.check(here, verb, path) {
        Ok(r) => r,
        Err(_) => return hint(&format!("The runtime would refuse it: {}.", why(w, here, verb, path))),
    };
    let entry = w.instance(r.inst).interface.serving(&r.path).cloned();
    let requests: Vec<_> = entry.iter().flat_map(|e| e.requests.iter().filter(|q| q.verb == verb).cloned()).collect();
    if requests.iter().all(|q| q.body.is_none()) {
        let answers: Vec<String> = requests.iter().filter_map(|q| q.answer.as_ref().map(|a| format!("{}{}", a.message.clone().unwrap_or(a.ty.clone()), if a.stream { "*" } else { "" }))).collect();
        let answers = if answers.is_empty() { String::new() } else { format!(" It answers {}.", answers.join(" or ")) };
        return hint(&format!("⏎ to send: {} takes no payload.{answers}", verb.keyword()));
    }
    let messages = &w.snapshot.messages;
    let src = source(w, r.inst);
    let rest_trimmed = rest.trim_start();
    let at = end - rest_trimmed.len();
    // The type
    if !rest_trimmed.contains(char::is_whitespace) && !rest_trimmed.starts_with('"') {
        let mut items = vec![];
        for q in &requests {
            let Some(ct) = &q.body else { continue };
            if let Some(m) = &q.message {
                let d = describe(&s.formats, messages, m);
                let fields = d.as_ref().map(|d| d.fields.iter().map(|f| format!("{}: {}", f.name, f.ty)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                items.push(Item::new(m, &format!("{m} {{ | }}")).detail(format!("{ct} message")).doc(format!("message {m} {{ {fields} }}"), d.map(|d| d.doc).unwrap_or_default(), &src));
            } else if ct.starts_with("text/") || ct == "*/*" {
                items.push(Item::new("\"text\"", "\"|\"").detail("text/plain").doc(r#""…""#, "A text/plain payload. Write another text type before it, as in text/html \"<p>hi</p>\".", "the image syntax"));
                if ct != "text/plain" && ct != "text/*" && ct != "*/*" {
                    items.push(Item::new(ct.clone(), &format!("{ct} \"|\"")).detail("content type").doc(ct.clone(), "", &src));
                }
            } else {
                items.push(Item::new(ct.clone(), &format!("{ct} from \"./|\"")).detail("a file's bytes").doc(format!("{ct} from \"./file\""), "The bytes of a file on this machine, relative to where the shell was started.", "the image syntax"));
            }
        }
        items.retain(|i| i.label.starts_with(rest_trimmed) || rest_trimmed.is_empty());
        items.dedup_by(|a, b| a.label == b.label);
        return Completion { from: at, items, ..Completion::default() };
    }
    // The payload
    let Some((ty, body)) = rest_trimmed.split_once(char::is_whitespace) else { return hint("⏎ to send.") };
    let Some(d) = describe(&s.formats, messages, ty) else { return hint("⏎ to send.") };
    let body_at = end - body.len();
    if !body.contains('{') {
        let item = Item::new("{ … }", "{ | }").detail(format!("{} fields", d.fields.len())).doc(format!("message {ty}"), d.doc.clone(), "its format");
        return Completion { from: body_at, items: vec![item], ..Completion::default() };
    }
    let given: Vec<&str> = body
        .split(|c| c == '{' || c == ',')
        .filter_map(|part| part.split_once(':').map(|(k, _)| k.trim()))
        .collect();
    let cut = body.rfind(|c| c == '{' || c == ',').map_or(0, |i| i + 1);
    let seg = &body[cut..];
    if let Some((field, value)) = seg.split_once(':') {
        let field = field.trim();
        let value = value.trim_start();
        let Some(f) = d.fields.iter().find(|f| f.name == field) else {
            return hint(&format!("{ty} has no field `{field}`; it has {}.", d.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")));
        };
        let items: Vec<Item> = f
            .examples
            .iter()
            .filter(|e| value.is_empty() || e.starts_with(value))
            .map(|e| {
                let insert = if e == "\"\"" { "\"|\"".to_string() } else { e.clone() };
                Item::new(e.clone(), &insert).detail(f.ty.clone()).doc(format!("{ty}.{}: {}", f.name, f.ty), f.doc.clone(), "its format")
            })
            .collect();
        let hint = items.is_empty().then(|| format!("{ty}.{} is a {}. Then `,` and the next field, or `}}`.", f.name, f.ty));
        return Completion { from: end - value.len(), items, hint, wants: None };
    }
    let stem = seg.trim_start();
    let items: Vec<Item> = d
        .fields
        .iter()
        .filter(|f| !given.contains(&f.name.as_str()) && f.name.starts_with(stem))
        .map(|f| Item::new(f.name.clone(), &format!("{}: ", f.name)).detail(f.ty.clone()).doc(format!("{ty}.{}: {}", f.name, f.ty), f.doc.clone(), "its format"))
        .collect();
    let hint = items.is_empty().then(|| format!("Every field of {ty} is given: close it with `}}` and ⏎."));
    Completion { from: end - stem.len(), items, hint, wants: None }
}

/// Message type `ty`, if one of the formats has it.
fn describe(formats: &Formats, messages: &AllSchemas, ty: &str) -> Option<MessageDoc> {
    formats.iter().find_map(|f| f.describe(messages.get(f.content_type())?, ty).ok())
}
