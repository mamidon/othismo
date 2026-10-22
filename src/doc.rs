//! Reference documentation generated from headers, as Markdown: what
//! `othismo doc` prints, and what the site's reference pages are made of.

use crate::iface::{self, Entry, Header, Object};
use crate::messages::MessageType;
use std::fmt::Write;

/// An object's page: its guide, or its `##` comment if it has none, then
/// the reference for what it serves and needs, and for `types`.
pub fn object(o: &Object, header: &Header, types: &[String]) -> String {
    let mut out = String::new();
    let guide = o.guide.as_deref().unwrap_or(&o.doc);
    if !guide.trim_start().starts_with("# ") {
        writeln!(out, "# {}\n", o.name).unwrap();
    }
    if !guide.trim().is_empty() {
        writeln!(out, "{}\n", guide.trim_end()).unwrap();
    }
    out.push_str("## Reference\n\n");
    reference(&mut out, o, "###");
    messages(&mut out, header, types, "###");
    out
}

/// The runtime's page: every built-in object, and the runtime's types.
pub fn env() -> String {
    let env = iface::env();
    let mut out = String::from(
        "# Built-in objects\n\n\
         The objects the runtime implements, which are the only way any object reaches the world \
         outside the image, and the message types the runtime and its objects use. They are \
         declared in the runtime's own header, `env`, and this page is generated from it.\n\n",
    );
    for o in &env.objects {
        writeln!(out, "## {}\n", o.name).unwrap();
        if !o.doc.is_empty() {
            writeln!(out, "{}\n", o.doc).unwrap();
        }
        reference(&mut out, o, "###");
    }
    messages(&mut out, env, &env.declared, "##");
    out
}

/// The message types an object's requests name, in order of first use.
pub fn types_used(o: &Object) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let entries = o.serves.iter().chain(o.needs.iter().flat_map(|n| &n.entries));
    for r in entries.flat_map(|e| &e.requests) {
        for ty in [r.body.as_deref(), r.answer.as_ref().map(|a| a.ty.as_str())].into_iter().flatten() {
            if ty != iface::OPAQUE && !out.iter().any(|t| t == ty) {
                out.push(ty.to_string());
            }
        }
    }
    out
}

fn reference(out: &mut String, o: &Object, h: &str) {
    if !o.traits.is_empty() {
        let traits: Vec<String> = o.traits.iter().map(|t| format!("`{t}`")).collect();
        writeln!(out, "Implements {}.\n", traits.join(", ")).unwrap();
    }
    if !o.serves.is_empty() {
        writeln!(out, "{h} Serves\n").unwrap();
        entries(out, &o.serves);
    }
    if !o.needs.is_empty() {
        writeln!(out, "{h} Needs\n").unwrap();
        for n in &o.needs {
            writeln!(out, "{h}# `{}`\n", n.at).unwrap();
            if !n.doc.is_empty() {
                writeln!(out, "{}\n", n.doc).unwrap();
            }
            entries(out, &n.entries);
        }
    }
}

fn entries(out: &mut String, entries: &[Entry]) {
    out.push_str("| Path | Requests | |\n|---|---|---|\n");
    for e in entries {
        let reqs: Vec<String> = e.requests.iter().map(|r| format!("`{r}`")).collect();
        writeln!(out, "| `{}` | {} | {} |", e.path, reqs.join(", "), cell(&e.doc)).unwrap();
    }
    out.push('\n');
}

fn messages(out: &mut String, header: &Header, types: &[String], h: &str) {
    let types: Vec<&MessageType> = types.iter().filter_map(|t| header.messages.get(t)).collect();
    if types.is_empty() {
        return;
    }
    writeln!(out, "{h} Messages\n").unwrap();
    for ty in types {
        writeln!(out, "{h}# `{}`\n", ty.name).unwrap();
        if !ty.doc.is_empty() {
            writeln!(out, "{}\n", ty.doc).unwrap();
        }
        if ty.fields.is_empty() {
            out.push_str("No fields.\n\n");
            continue;
        }
        out.push_str("| Field | Type | |\n|---|---|---|\n");
        for ((name, fty), doc) in ty.fields.iter().zip(&ty.field_docs) {
            writeln!(out, "| `{name}` | {fty} | {} |", cell(doc)).unwrap();
        }
        out.push('\n');
    }
}

/// Documentation for a table cell: one line, with `|` escaped.
fn cell(doc: &str) -> String {
    doc.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(src: &str) -> Header {
        let file = iface::parse(src).unwrap();
        iface::resolve(&file, &mut |_| Ok(iface::env().clone()), &|_| Ok("# The Guide\n\nAll about it.".into())).unwrap()
    }

    const SITE: &str = r#"
        import { Data, Info } from env
        ## A count.
        message Hits {
          ## So far; never | less.
          n: u64
        }
        ## Serves a site.
        ##
        ## From its files.
        object Site {
          ## The files.
          needs /files { /** read -> Data, stat -> Info }
          serves {
            ## How many,
            ## so far.
            /hits  read -> Hits
            /**    read -> Data
          }
        }
    "#;

    #[test]
    fn documents_an_object_from_its_header() {
        let h = header(SITE);
        let out = object(&h.objects[0], &h, &h.declared);
        assert!(out.starts_with("# Site\n\nServes a site.\n\nFrom its files.\n\n## Reference\n\n### Serves\n\n"), "{out}");
        assert!(out.contains("| `/hits` | `read -> Hits` | How many, so far. |\n| `/**` | `read -> Data` |  |\n"), "{out}");
        assert!(out.contains("### Needs\n\n#### `/files`\n\nThe files.\n\n| Path | Requests | |"), "{out}");
        assert!(out.contains("### Messages\n\n#### `Hits`\n\nA count.\n\n| Field | Type | |\n|---|---|---|\n| `n` | u64 | So far; never \\| less. |"), "{out}");
        assert!(!out.contains("`Data`\n"), "imported types are documented where they are declared");
    }

    #[test]
    fn a_guide_replaces_the_comment() {
        let h = header(r#"object Big { guide "./Big.md" }"#);
        assert_eq!(object(&h.objects[0], &h, &[]), "# The Guide\n\nAll about it.\n\n## Reference\n\n");
    }

    #[test]
    fn documents_the_built_in_objects() {
        let out = env();
        assert!(out.starts_with("# Built-in objects\n\n"), "{out}");
        for o in ["## Environment.Console", "## Environment.Tcp", "## Packed.Files", "## Messages", "### `Accepted`"] {
            assert!(out.contains(&format!("{o}\n")), "{o}: {out}");
        }
        let console = iface::env().object("Environment.Console").unwrap();
        assert_eq!(types_used(console), ["Line"]);
    }
}
