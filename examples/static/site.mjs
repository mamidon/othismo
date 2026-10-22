// Renders the project's docs into the static example's site:
//
//   docs/overview.md   -> public/overview/index.html
//   docs/objects.md    -> public/objects/index.html
//   docs/namespace.md  -> public/docs/index.html
//   docs/images.md     -> public/images/index.html
//
// Each page takes its title from the document's `# heading` and its lede
// from the first paragraph, and gets a contents list of its `##` sections.
// Run with `task build:docs`; the output is committed so the example needs no build.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { Marked } from "marked"

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, "..", "..")

const PAGES = [
  {
    source: "docs/overview.md",
    out: "overview/index.html",
    nav: "/overview/",
    eyebrow: "Overview",
    description: "What Othismo is and why: sandboxed objects, namespaces and images you can review instead of the code.",
  },
  {
    source: "docs/objects.md",
    out: "objects/index.html",
    nav: "/objects/",
    eyebrow: "Objects",
    description: "Objects: the sandboxed modules you don't read, and the interfaces you do.",
  },
  {
    source: "docs/namespace.md",
    out: "docs/index.html",
    nav: "/docs/",
    eyebrow: "Design",
    description: "The namespace design: views, verbs, the AssemblyScript SDK, environment objects and the host ABI.",
  },
  {
    source: "docs/images.md",
    out: "images/index.html",
    nav: "/images/",
    eyebrow: "Building",
    description: "Building images: sealing an image into one file, packed files, and slots the host fills in.",
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
  "docs/objects.md": "/objects/",
  "objects.md": "/objects/",
}

function slug(text) {
  return text.toLowerCase().replace(/<[^>]+>/g, "").replace(/&[a-z]+;/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")
}

function escape(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;")
}

function render(markdown) {
  const toc = []
  const marked = new Marked({
    renderer: {
      heading({ tokens, depth }) {
        const html = this.parser.parseInline(tokens)
        const id = slug(html)
        if (depth === 2) toc.push({ id, html })
        return `<h${depth} id="${id}">${html}</h${depth}>\n`
      },
      link({ href, title, tokens }) {
        const text = this.parser.parseInline(tokens)
        const target = LINKS[href] ?? href
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
          ${navLink("/objects/", "Objects")}
          ${navLink("/docs/", "Design")}
          ${navLink("/images/", "Building")}
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
  const doc = render(readFileSync(join(root, p.source), "utf8"))
  const out = join(here, "public", p.out)
  mkdirSync(dirname(out), { recursive: true })
  writeFileSync(out, page(p, doc))
  console.log(`${p.source} -> ${join("examples/static/public", p.out)}`)
}
