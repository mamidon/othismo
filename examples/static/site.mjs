// Renders the project's docs into the static example's site:
//
//   docs/overview.md   -> public/overview/index.html
//   docs/tour.md       -> public/tour/index.html
//   docs/objects.md    -> public/objects/index.html
//   docs/protocol.md   -> public/protocol/index.html
//   docs/namespace.md  -> public/docs/index.html
//   docs/images.md     -> public/images/index.html
//   `othismo doc env`  -> public/reference/index.html
//
// Each page takes its title from the document's `# heading` and its lede
// from the first paragraph, and gets a contents list of its `##` sections.
// The reference is generated from the runtime's header by othismo itself:
// $OTHISMO, or target/debug/othismo, or the one on the PATH.
// Run with `task build:docs`; the output is committed so the example needs no build.

import { execFileSync } from "node:child_process"
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { Marked } from "marked"

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, "..", "..")
const built = join(root, "target", "debug", "othismo")
const OTHISMO = process.env.OTHISMO ?? (existsSync(built) ? built : "othismo")

const PAGES = [
  {
    source: "docs/overview.md",
    out: "overview/index.html",
    nav: "/overview/",
    eyebrow: "Overview",
    description: "What Othismo is and why: sandboxed objects, namespaces and images you can review instead of the code.",
  },
  {
    source: "docs/tour.md",
    out: "tour/index.html",
    nav: "/tour/",
    eyebrow: "Tour",
    description: "A tour of the image serving this site, file by file.",
  },
  {
    source: "docs/objects.md",
    out: "objects/index.html",
    nav: "/objects/",
    eyebrow: "Objects",
    description: "Objects: the sandboxed modules you don't read, and the interfaces you do.",
  },
  {
    source: "docs/protocol.md",
    out: "protocol/index.html",
    nav: "/protocol/",
    eyebrow: "Protocol",
    description: "The three layers: the namespace, the protocol every object speaks, and payload formats; the host ABI and the SDK.",
  },
  {
    source: "docs/namespace.md",
    out: "docs/index.html",
    nav: "/docs/",
    eyebrow: "Namespaces",
    description: "Namespaces: views and served trees, binds and exports, environment objects as needs, and what the checker can promise.",
  },
  {
    source: "docs/images.md",
    out: "images/index.html",
    nav: "/images/",
    eyebrow: "Building",
    description: "The .oth language: imports and exports, images as objects, slots, packed files, testing, and sealing an image into one file.",
  },
  {
    source: "sdk/env/",
    doc: "env",
    out: "reference/index.html",
    nav: "/reference/",
    eyebrow: "Reference",
    description: "The runtime's objects and the shapes they are made of, generated from the runtime's header.",
  },
]

/** Where links between the docs point on the site. */
const LINKS = {
  "docs/namespace.md": "/docs/",
  "namespace.md": "/docs/",
  "docs/images.md": "/images/",
  "images.md": "/images/",
  "docs/overview.md": "/overview/",
  "overview.md": "/overview/",
  "docs/tour.md": "/tour/",
  "tour.md": "/tour/",
  "docs/objects.md": "/objects/",
  "objects.md": "/objects/",
  "docs/protocol.md": "/protocol/",
  "protocol.md": "/protocol/",
}

/** A heading's id, as GitHub makes it, so links work in both places. */
function slug(text) {
  return text.toLowerCase().replace(/<[^>]+>/g, "").replace(/&[#a-z0-9]+;/g, "").replace(/[^a-z0-9 _-]/g, "").trim().replace(/ /g, "-")
}

function escape(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;")
}

function render(markdown) {
  const toc = []
  const ids = new Map()
  const marked = new Marked({
    renderer: {
      heading({ tokens, depth }) {
        const html = this.parser.parseInline(tokens)
        // Headings repeat in the reference ("Serves"), and ids mustn't.
        const base = slug(html)
        const n = (ids.get(base) ?? 0) + 1
        ids.set(base, n)
        const id = n === 1 ? base : `${base}-${n}`
        if (depth === 2) toc.push({ id, html })
        return `<h${depth} id="${id}">${html}</h${depth}>\n`
      },
      link({ href, title, tokens }) {
        const text = this.parser.parseInline(tokens)
        const [path, fragment] = href.split("#")
        const target = path in LINKS ? LINKS[path] + (fragment ? `#${fragment}` : "") : href
        return `<a href="${escape(target)}"${title ? ` title="${escape(title)}"` : ""}>${text}</a>`
      },
    },
  })

  // The leading `# Title` and first paragraph become the page header.
  const tokens = marked.lexer(markdown)
  const first = tokens.findIndex((t) => t.type !== "space")
  const title = tokens[first]
  if (title?.type !== "heading" || title.depth !== 1) throw new Error("document must start with a # heading")
  const rest = tokens.slice(first + 1)
  const ledeAt = rest.findIndex((t) => t.type !== "space")
  const lede = rest[ledeAt]?.type === "paragraph" ? rest.splice(ledeAt, 1)[0] : null
  rest.links = tokens.links

  const body = marked.parser(rest)
    .replace(/<table>/g, '<div class="table"><table>')
    .replace(/<\/table>/g, "</table></div>")
  return {
    title: marked.parseInline(title.text),
    lede: lede ? marked.parseInline(lede.text) : "",
    body,
    toc,
  }
}

function page(p, doc) {
  const plainTitle = doc.title.replace(/<[^>]+>/g, "")
  const navLink = (href, label) =>
    `<a class="nav-link" href="${href}"${href === p.nav ? ' aria-current="page"' : ""}>${label}</a>`
  const toc = doc.toc.map((h) => `          <li><a href="#${h.id}">${h.html}</a></li>`).join("\n")
  return `<!doctype html>
<!-- Generated from ${p.source} by examples/static/site.mjs (task build:docs). Edit the source, not this file. -->
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escape(plainTitle)} — ${p.eyebrow} — othismo</title>
  <meta name="description" content="${escape(p.description)}">
  <link rel="icon" type="image/svg+xml" href="/assets/mark-orange.svg">
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Onest:wght@400;600;700;800&family=Asap:ital,wght@0,400;0,500;0,600;1,600&family=Antonio:wght@500;600&display=swap" rel="stylesheet">
  <link rel="stylesheet" href="/style.css">
</head>
<body>

  <header class="hero doc">
    <div class="wrap">
      <nav class="nav">
        <a href="/"><img src="/assets/logo-full-orange.svg" alt="Othismo — home"></a>
        <div class="nav-links">
          ${navLink("/overview/", "Overview")}
          ${navLink("/reference/", "Reference")}
        </div>
      </nav>

      <div class="hero-body">
        <div class="eyebrow">${p.eyebrow}</div>
        <h1>${doc.title}</h1>
        <p class="lede">${doc.lede}</p>
      </div>
    </div>
  </header>

  <main class="wrap doc-layout">
    <aside class="toc">
      <nav aria-label="Contents">
        <div class="eyebrow">Contents</div>
        <ol>
${toc}
        </ol>
      </nav>
    </aside>

    <article class="prose">
${doc.body}
    </article>
  </main>

  <footer>
    <div class="wrap">
      <a href="/"><img src="/assets/logo.png" alt="Othismo — home"></a>
      <small>othismo &middot; served by othismo</small>
    </div>
  </footer>
</body>
</html>
`
}

for (const p of PAGES) {
  const markdown = p.doc
    ? execFileSync(OTHISMO, ["doc", p.doc], { encoding: "utf8" })
    : readFileSync(join(root, p.source), "utf8")
  const doc = render(markdown)
  const out = join(here, "public", p.out)
  mkdirSync(dirname(out), { recursive: true })
  writeFileSync(out, page(p, doc))
  console.log(`${p.doc ? `othismo doc ${p.doc}` : p.source} -> ${join("examples/static/public", p.out)}`)
}
