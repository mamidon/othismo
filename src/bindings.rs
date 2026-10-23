//! Generates AssemblyScript bindings from a header: a server base class for
//! each object, which routes the requests it serves to a method each, and a
//! client for each of its needs, whose methods make the requests they list.
//! A schema file's bindings are its format's to generate.

use crate::iface::{self, Entry, Header};
use crate::ns::Verb;
use std::collections::BTreeSet;
use std::fmt::Write;

/// A JSDoc comment, if there is anything to say. A `*/` in it, as in
/// `*/*`, would end it, so it is written `*\/`.
fn doc(out: &mut String, indent: &str, text: &str) {
    let text = text.replace("*/", "*\\/");
    match text.lines().collect::<Vec<_>>().as_slice() {
        [] => {}
        [one] => writeln!(out, "{indent}/** {one} */").unwrap(),
        lines => {
            writeln!(out, "{indent}/**").unwrap();
            for line in lines {
                writeln!(out, "{indent} *{}{line}", if line.is_empty() { "" } else { " " }).unwrap();
            }
            writeln!(out, "{indent} */").unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// From a header

/// The bindings for `header`: a server base for each object it declares,
/// and a client for each of their needs. `source` names the header in the
/// generated comment; `library` and `env` are where the SDK and the
/// runtime's clients are imported from. `None` if there is nothing to bind.
pub fn namespace(header: &Header, source: &str, library: &str, env: &str) -> Option<String> {
    if header.objects.is_empty() {
        return None;
    }
    let mut body = String::new();
    let mut from_env: BTreeSet<String> = BTreeSet::new();
    let mut classes: BTreeSet<String> = BTreeSet::new();
    let mut values: BTreeSet<String> = BTreeSet::new();
    for o in &header.objects {
        body.push('\n');
        server(&mut body, &format!("{}Server", pascal(&o.name)), &o.doc, &o.serves);
        for n in &o.needs {
            let class = match &n.shape {
                Some(t) if iface::env().traits.contains_key(t) => {
                    from_env.insert(format!("{}Client", pascal(t)));
                    format!("{}Client", pascal(t))
                }
                shape => {
                    let name = shape.clone().unwrap_or_else(|| need_name(n.at.as_str()));
                    let class = format!("{}Client", pascal(&name));
                    if classes.insert(class.clone()) {
                        body.push('\n');
                        client(&mut body, &class, &n.doc, &n.entries);
                    }
                    class
                }
            };
            if n.at.is_literal() {
                let mut value = camel(&need_name(n.at.as_str()));
                while !values.insert(value.clone()) {
                    value.push('_');
                }
                body.push('\n');
                doc(&mut body, "", &n.doc);
                writeln!(body, "export const {value} = new {class}(\"{}\")", n.at).unwrap();
            }
        }
    }
    let mut out = format!(
        "// Generated from {source} by `othismo bindings`. Edit the header, not this file.\n\n\
         import {{ Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under }} from \"{library}\"\n"
    );
    if !from_env.is_empty() {
        writeln!(out, "import {{ {} }} from \"{env}\"", from_env.into_iter().collect::<Vec<_>>().join(", ")).unwrap();
    }
    out.push_str(&body);
    Some(out)
}

/// The runtime's clients and server bases: one of each for every trait in
/// its header, which modules import as `"othismo/env"`. `library` is where
/// the SDK is imported from.
pub fn env(library: &str) -> String {
    let mut out = format!(
        "// Generated from env by `othismo bindings`. Edit the runtime's header, not this file.\n\n\
         import {{ Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under }} from \"{library}\"\n"
    );
    for t in iface::env().traits.values() {
        out.push('\n');
        client(&mut out, &format!("{}Client", pascal(&t.name)), &t.doc, &t.entries);
        out.push('\n');
        server(&mut out, &format!("{}Server", pascal(&t.name)), &t.doc, &t.entries);
    }
    out
}

/// What a method takes besides the request: no payload, text of one type,
/// a payload of one type, or whatever comes.
enum Takes {
    Nothing,
    Text(String),
    Payload(String),
    Anything,
}

/// One request an entry lists, as a method.
struct Method {
    name: String,
    verb: Verb,
    /// The path's `{name}` segments, and `rest` for a `**`.
    params: Vec<String>,
    /// The path below the mount point, as an AssemblyScript expression.
    rel: String,
    takes: Takes,
    /// The entry's documentation, and what it takes: `write /greeting application/bson`.
    doc: String,
}

/// The methods of `entries`, the most specific path first, each named for
/// its verb and the literal segments of its path: `writeGreeting` for
/// `/greeting`, `readAny(rest)` for `/**`, `watchData(conn)` for
/// `/{conn}/data`, and the verb alone for `/`.
fn methods(entries: &[Entry]) -> Vec<(&Entry, Vec<Method>)> {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| b.path.specificity(&a.path));
    let mut names: BTreeSet<String> = BTreeSet::new();
    sorted
        .into_iter()
        .map(|e| {
            let (stem, params, rel) = path_parts(e.path.as_str());
            let mut verbs: Vec<Verb> = vec![];
            for r in &e.requests {
                if !verbs.contains(&r.verb) {
                    verbs.push(r.verb);
                }
            }
            let methods = verbs
                .into_iter()
                .map(|verb| {
                    let reqs: Vec<&iface::Request> = e.requests.iter().filter(|r| r.verb == verb).collect();
                    let bodies: BTreeSet<Option<&str>> = reqs.iter().map(|r| r.body.as_deref()).collect();
                    let takes = match bodies.into_iter().collect::<Vec<_>>().as_slice() {
                        [None] => Takes::Nothing,
                        [Some(t)] if is_text(t) => Takes::Text(t.to_string()),
                        [Some(t)] => Takes::Payload(t.to_string()),
                        _ => Takes::Anything,
                    };
                    let mut name = format!("{}{stem}", verb.keyword());
                    if !names.insert(name.clone()) {
                        let mut n = 2;
                        while !names.insert(format!("{name}{n}")) {
                            n += 1;
                        }
                        name = format!("{name}{n}");
                    }
                    let what: Vec<String> = reqs.iter().map(|r| format!("`{} {r}`", e.path)).collect();
                    let doc = if e.doc.is_empty() { what.join(", ") } else { format!("{}\n\n{}", e.doc, what.join(", ")) };
                    Method { name, verb, params: params.clone(), rel: rel.clone(), takes, doc }
                })
                .collect();
            (e, methods)
        })
        .collect()
}

/// A path pattern's method name stem, its parameters, and the path below
/// the mount point, as an expression.
fn path_parts(pattern: &str) -> (String, Vec<String>, String) {
    let segs: Vec<&str> = pattern.strip_prefix('/').unwrap_or(pattern).split('/').collect();
    let (mut stem, mut params, mut parts) = (String::new(), vec![], vec![]);
    let mut lit = String::new();
    for seg in &segs {
        match *seg {
            "" if segs.len() == 1 => {}
            "" => {
                stem.push_str("Index");
                lit.push('/');
            }
            "**" => {
                stem.push_str("Any");
                params.push("rest".to_string());
                if !lit.is_empty() {
                    parts.push(format!("{lit:?}"));
                    lit.clear();
                }
                parts.push("rest".into());
            }
            s if s.starts_with('{') => {
                let p = camel(&s[1..s.len() - 1]);
                lit.push('/');
                parts.push(format!("{lit:?}"));
                lit.clear();
                parts.push(p.clone());
                params.push(p);
            }
            s => {
                stem.push_str(&pascal(s));
                lit.push('/');
                lit.push_str(s);
            }
        }
    }
    if !lit.is_empty() || parts.is_empty() {
        parts.push(format!("{lit:?}"));
    }
    (stem, params, parts.join(" + "))
}

/// A server base class: a method for each request, which fails until the
/// module overrides it, and `serve`, which routes to them.
fn server(out: &mut String, class: &str, about: &str, entries: &[Entry]) {
    doc(out, "", &first_paragraph(about));
    writeln!(out, "export class {class} extends Server {{").unwrap();
    let all = methods(entries);
    for (_, methods) in &all {
        for m in methods {
            let mut args: Vec<String> = vec!["req: Request".into()];
            args.extend(m.params.iter().map(|p| format!("{p}: string")));
            match &m.takes {
                Takes::Nothing => {}
                Takes::Text(_) => args.push("body: string".into()),
                Takes::Payload(_) => args.push("body: Payload".into()),
                Takes::Anything => args.push("body: Payload | null".into()),
            }
            doc(out, "  ", &m.doc);
            writeln!(out, "  {}({}): void {{ req.fail(Errno.NotSupported) }}\n", m.name, args.join(", ")).unwrap();
        }
    }
    out.push_str("  serve(req: Request): void {\n");
    for (i, (e, methods)) in all.iter().enumerate() {
        writeln!(out, "    const m{i} = route(\"{}\", req.path)", e.path).unwrap();
        writeln!(out, "    if (m{i} !== null) {{").unwrap();
        for m in methods {
            let mut args: Vec<String> = vec!["req".into()];
            args.extend((0..m.params.len()).map(|k| format!("m{i}![{k}]")));
            let verb = verb(m.verb);
            match &m.takes {
                Takes::Nothing => writeln!(out, "      if (req.verb == Verb.{verb}) {{ this.{}({}); return }}", m.name, args.join(", ")),
                Takes::Text(t) | Takes::Payload(t) => {
                    let of = if matches!(m.takes, Takes::Text(_)) { "textOf" } else { "payloadOf" };
                    args.push("body!".into());
                    writeln!(
                        out,
                        "      if (req.verb == Verb.{verb}) {{ const body = {of}(req, \"{t}\"); if (body !== null) this.{}({}); return }}",
                        m.name,
                        args.join(", ")
                    )
                }
                Takes::Anything => {
                    args.push("req.body".into());
                    writeln!(out, "      if (req.verb == Verb.{verb}) {{ this.{}({}); return }}", m.name, args.join(", "))
                }
            }
            .unwrap();
        }
        out.push_str("      req.fail(Errno.NotSupported)\n      return\n    }\n");
    }
    out.push_str("    req.fail(Errno.NotFound)\n  }\n}\n");
}

/// A client: a method for each request `entries` list, made of the mount
/// point it is constructed with. Objects can't make the operator's
/// requests, so it has no methods for those.
fn client(out: &mut String, class: &str, about: &str, entries: &[Entry]) {
    doc(out, "", &first_paragraph(about));
    writeln!(out, "export class {class} {{\n  constructor(public at: string) {{}}").unwrap();
    let entries: Vec<Entry> = entries
        .iter()
        .map(|e| Entry { requests: e.requests.iter().filter(|r| !r.operator).cloned().collect(), ..e.clone() })
        .filter(|e| !e.requests.is_empty())
        .collect();
    for (_, methods) in methods(&entries) {
        for m in methods {
            let mut args: Vec<String> = m.params.iter().map(|p| format!("{p}: string")).collect();
            let payload = match &m.takes {
                Takes::Nothing => "null".to_string(),
                Takes::Text(t) => {
                    args.push("body: string".into());
                    format!("Payload.text(body, \"{t}\")")
                }
                Takes::Payload(_) => {
                    args.push("body: Payload".into());
                    "body".into()
                }
                Takes::Anything => {
                    args.push("body: Payload | null".into());
                    "body".into()
                }
            };
            args.push(if m.verb.is_safe() { "then: Then".into() } else { "then: Then | null = null".into() });
            out.push('\n');
            doc(out, "  ", &m.doc);
            writeln!(
                out,
                "  {}({}): Call {{\n    return ns.call(Verb.{}, under(this.at, {}), {payload}, then)\n  }}",
                m.name,
                args.join(", "),
                verb(m.verb),
                m.rel
            )
            .unwrap();
        }
    }
    out.push_str("}\n");
}

fn verb(v: Verb) -> String {
    pascal(v.keyword())
}

/// Whether payloads of `ty` are text of that one type, so a method can
/// take and give a string.
fn is_text(ty: &str) -> bool {
    ty.to_ascii_lowercase().starts_with("text/") && !ty.split(';').next().unwrap_or("").ends_with("/*")
}

/// What a need's client and value are named for: its mount point's last
/// literal segment, or `Any` for one under a pattern.
fn need_name(at: &str) -> String {
    match at.rsplit('/').find(|s| !s.is_empty() && !s.starts_with('{') && *s != "**") {
        Some(s) if !at.contains("**") && !at.contains('{') => s.to_string(),
        _ => "any".into(),
    }
}

/// `words-like.this` as `WordsLikeThis`.
fn pascal(s: &str) -> String {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

/// `words-like.this` as `wordsLikeThis`.
fn camel(s: &str) -> String {
    let p = pascal(s);
    let mut c = p.chars();
    c.next().map(|f| f.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

fn first_paragraph(doc: &str) -> String {
    doc.split("\n\n").next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtimes_bindings_are_up_to_date() {
        assert!(env("./index") == include_str!("../as/env.ts"), "as/env.ts is stale; run `task bindings`");
    }

    #[test]
    fn generates_a_server_base_and_clients_from_a_header() {
        let src = r#"
            import { Console } from env
            ## Greets.
            ##
            ## At length.
            export object Greeter {
              needs /dev/cons Console
              ## Something to shout with.
              needs /shouter { / write text/plain }
              needs /sites/** { /** read -> */*, stat }
              serves {
                ## How to greet.
                /greeting       write application/bson
                /hosts/{name}   operator write text/plain, operator remove, read -> text/plain
                /hosts/         list
                /**             read -> */*
              }
            }
        "#;
        let file = iface::parse(src).unwrap();
        let header = iface::resolve(&file, &mut |_| Ok(iface::env().clone()), &|_| unreachable!()).unwrap();
        let out = namespace(&header, "Greeter.oth", "othismo", "othismo/env").unwrap();
        assert!(out.contains("import { ConsoleClient } from \"othismo/env\"\n"), "{out}");
        assert!(out.contains("/** Greets. */\nexport class GreeterServer extends Server {"), "{out}");
        assert!(out.contains("  writeGreeting(req: Request, body: Payload): void { req.fail(Errno.NotSupported) }"), "{out}");
        assert!(out.contains("  writeHosts(req: Request, name: string, body: string): void"), "{out}");
        assert!(out.contains("  removeHosts(req: Request, name: string): void"), "{out}");
        assert!(out.contains("  listHostsIndex(req: Request): void"), "{out}");
        assert!(out.contains("  readAny(req: Request, rest: string): void"), "{out}");
        assert!(out.contains("const body = payloadOf(req, \"application/bson\"); if (body !== null) this.writeGreeting(req, body!)"), "{out}");
        assert!(out.contains("const body = textOf(req, \"text/plain\"); if (body !== null) this.writeHosts(req, m1![0], body!)"), "{out}");
        assert!(out.find("route(\"/greeting\"").unwrap() < out.find("route(\"/**\"").unwrap(), "the most specific first");
        assert!(out.contains("export class ShouterClient {\n  constructor(public at: string) {}"), "{out}");
        assert!(out.contains("  write(body: string, then: Then | null = null): Call {\n    return ns.call(Verb.Write, under(this.at, \"\"), Payload.text(body, \"text/plain\"), then)\n  }"), "{out}");
        assert!(out.contains("export class AnyClient {"), "{out}");
        assert!(out.contains("  readAny(rest: string, then: Then): Call {\n    return ns.call(Verb.Read, under(this.at, rest), null, then)"), "{out}");
        assert!(out.contains("export const cons = new ConsoleClient(\"/dev/cons\")"), "{out}");
        assert!(out.contains("/** Something to shout with. */\nexport const shouter = new ShouterClient(\"/shouter\")"), "{out}");
        assert!(!out.contains("new AnyClient("), "a need under a pattern has no client made");
        assert_eq!(path_parts("/{conn}/data"), ("Data".into(), vec!["conn".into()], "\"/\" + conn + \"/data\"".into()));
        assert_eq!(path_parts("/hosts/"), ("HostsIndex".into(), vec![], "\"/hosts/\"".into()));
        assert_eq!(path_parts("/"), ("".into(), vec![], "\"\"".into()));
    }
}
