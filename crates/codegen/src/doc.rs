//! Reference documentation generated from headers, as Markdown: what
//! `othismo doc` prints, and what the site's reference pages are made of.

use compiler::check;
use compiler::iface::{self, Entry, Object};
use format::{AllSchemas, Formats};
use std::fmt::Write;

/// An object's page: its guide, or its `##` comment if it has none, then
/// the reference for what it serves and needs, and for the types its
/// module declares in each of `formats`.
pub fn object(o: &Object, formats: &Formats, schemas: &AllSchemas) -> String {
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
    for f in formats.iter() {
        if let Some(s) = schemas.get(f.content_type()) {
            out.push_str(&f.document(s, "###"));
        }
    }
    out
}

/// The runtime's page: the traits it runs, Packed.Files, the shapes they
/// are made of, and the runtime's types.
pub fn env() -> String {
    let env = iface::env();
    let mut out = String::from(
        "# Built-in objects\n\n\
         The objects the runtime implements, and the shapes they are made of. Console, Listener and \
         Files are the only way any object reaches the world outside the image: the image being run \
         gets one by needing it, `needs /net/tcp Listener`, and the runtime picks how to run it. The \
         others touch nothing outside the image, and any image can declare one by its name. They are \
         declared in the runtime's own header, `env`, and this page is generated from it.\n\n",
    );
    let runs = check::NATIVE.iter().map(|name| env.traits[*name].as_object()).chain(env.objects.iter().cloned());
    for o in runs {
        writeln!(out, "## {}\n", o.name).unwrap();
        if !o.doc.is_empty() {
            writeln!(out, "{}\n", o.doc).unwrap();
        }
        reference(&mut out, &o, "###");
    }
    let shapes: Vec<_> = env.traits.values().filter(|t| !check::NATIVE.contains(&t.name.as_str())).collect();
    if !shapes.is_empty() || !env.protocols.is_empty() {
        out.push_str("## Shapes\n\nTraits and protocols the objects above are made of, for headers to use.\n\n");
        for t in shapes {
            writeln!(out, "### `{}`\n", t.name).unwrap();
            if !t.doc.is_empty() {
                writeln!(out, "{}\n", t.doc).unwrap();
            }
            entries(&mut out, &t.entries);
        }
        out.push_str("### Protocols\n\n| Protocol | Requests | |\n|---|---|---|\n");
        for p in env.protocols.values() {
            let reqs: Vec<String> = p.requests.iter().map(|r| format!("`{r}`")).collect();
            writeln!(out, "| `{}` | {} | {} |", p.name, reqs.join(", "), cell(&p.doc)).unwrap();
        }
        out.push('\n');
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

/// Documentation for a table cell: one line, with `|` escaped.
fn cell(doc: &str) -> String {
    doc.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use format::Format;
    use format_bson::{BSON, Bson};

    fn formats() -> Formats {
        Formats::new(vec![Box::new(Bson)])
    }

    fn header(src: &str) -> iface::Header {
        let file = iface::parse(src).unwrap();
        iface::resolve(&file, &mut |_| Ok(iface::env().clone()), &|_| Ok("# The Guide\n\nAll about it.".into())).unwrap()
    }

    const HITS: &str = r#"
        ## A count.
        message Hits {
          ## So far; never | less.
          n: u64
        }
    "#;

    const SITE: &str = r#"
        ## Serves a site.
        ##
        ## From its files.
        object Site {
          ## The files.
          needs /files { /** read -> */*, stat }
          serves {
            ## How many,
            ## so far.
            /hits  read -> application/bson
            /**    read -> */*
          }
        }
    "#;

    #[test]
    fn documents_an_object_from_its_header() {
        let h = header(SITE);
        let schemas = AllSchemas::from([(BSON.to_string(), Bson.parse(HITS, &mut |_| unreachable!()).unwrap())]);
        let out = object(&h.objects[0], &formats(), &schemas);
        assert!(out.starts_with("# Site\n\nServes a site.\n\nFrom its files.\n\n## Reference\n\n### Serves\n\n"), "{out}");
        assert!(out.contains("| `/hits` | `read -> application/bson` | How many, so far. |\n| `/**` | `read -> */*` |  |\n"), "{out}");
        assert!(out.contains("### Needs\n\n#### `/files`\n\nThe files.\n\n| Path | Requests | |"), "{out}");
        assert!(out.contains("### Messages\n\nPayloads of `application/bson`, each named by its `$type`.\n\n#### `Hits`\n\nA count.\n\n| Field | Type | |\n|---|---|---|\n| `n` | u64 | So far; never \\| less. |"), "{out}");
    }

    #[test]
    fn a_guide_replaces_the_comment() {
        let h = header(r#"object Big { guide "./Big.md" }"#);
        assert_eq!(object(&h.objects[0], &formats(), &AllSchemas::new()), "# The Guide\n\nAll about it.\n\n## Reference\n\n");
    }

    #[test]
    fn documents_the_built_in_objects() {
        let out = env();
        assert!(out.starts_with("# Built-in objects\n\n"), "{out}");
        for o in ["## Console", "## Listener", "## Files", "## PackedFiles", "## MemoryListener", "## Shapes", "### `FileTree`"] {
            assert!(out.contains(&format!("{o}\n")), "{o}: {out}");
        }
        assert!(out.contains("| `Connection` | `watch -> application/octet-stream*`, `write application/octet-stream` |"), "{out}");
    }
}
