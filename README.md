# real_talk

A runtime for **images**: declarative sets of sandboxed AssemblyScript
objects that reach each other only through paths in their own namespaces. If
the image doesn't put something in an object's view, the object can't reach
it. Environment objects (console, TCP, files) are the only way out to the
world.

The model is described in [docs/namespace.md](docs/namespace.md). This README
covers using it.

## Quick start

Requires Rust and Node.

```sh
make hello            # npm install, asc → scripts/*.wasm, cargo run -- examples/hello/hello.img
make greeter          # a larger example
make static           # serves these docs as a site on http://127.0.0.1:8080
make test             # build the wasm, then cargo test
```

After `make wasm`, run any image directly:

```sh
cargo run -- examples/hello/hello.img
cargo run -- --check image.img          # validate, print every view and what each instance can reach
cargo run -- --check-shapes image.img   # also check every body's fields in transit
```

## Layout

| Path | What |
|---|---|
| `src/image.rs` | Lexer and parser for `.img` files |
| `src/messages.rs` | Minimal reader for `export class X { field: type }` in AssemblyScript |
| `src/check.rs` | Resolves imports, mounts, config and startup requests → `Program` |
| `src/ns.rs` | Paths, verbs, statuses, views, exposures and the static reach analysis |
| `src/wire.rs` | BSON wire format |
| `src/runtime.rs` | Wasmtime host, the host ABI and the message loop |
| `src/env.rs` | Environment objects: Console, Tcp, Files |
| `src/render.rs` | How Console prints a message |
| `as/index.ts` | `"real-talk"`: `Message`, `Writer`, `Reader`, `registerMessage` |
| `as/ns.ts` | `"real-talk/ns"`: the namespace SDK |
| `as/env.ts` | `"real-talk/env"`: the runtime's message types |
| `examples/` | `hello` (the acceptance image), `greeter`, `static` (HTTP server) |
| `examples/static/site.mjs` | Renders this README and the design doc into the static example's site (`make site`) |
| `docs/namespace.md` | The design: model, verbs, SDK, ABI |

## The image language

An image declares instances. Each instance block holds that instance's view
(`mount`), environment config (`send`), and requests the image makes of it at
startup (a verb and a path).

```text
import { Greeting, Stats } from "./messages"   // types the image or its objects use

instance cons : Environment.Console

instance shouter : Shouter {                   // runs scripts/Shouter.wasm
  mount cons /dev/cons
}

instance greeter : Greeter {
  mount cons /dev/cons                         // greeter's view: two mounts
  mount shouter /shouter
  write /greeting Greeting { prefix: "Hello, ", excited: true }
}
```

- `mount <instance> <path> [readonly]` puts the instance's whole served tree
  at `<path>` in this instance's view. With `readonly`, only the safe verbs
  (`read`, `stat`, `list`, `watch`) get through.
- `<verb> <path> [Type { fields }]` is a startup request to this instance, at
  `<path>` in its own served tree. The verbs are `read`, `stat`, `list`,
  `watch`, `write`, `create`, `remove` and `invoke`. Replies are dropped, but
  a failure is printed to stderr as a warning.
- `send Type { fields }` configures an environment object, and only that.
- The runtime's message types (`Line`, `Text`, `Data`, `Info`, `Entry`,
  `Accepted`, `Listen`, `FilesRoot`) are always in scope. Other types are
  imported from AssemblyScript files. With `--check-shapes`, every type an
  object sends must be one the image knows.

The checker rejects an image when:

- an imported name isn't an exported class in the referenced file, a field has
  a type other than `string`, `bool`, `i8`–`i64`, `u8`–`u64`, `f32`, `f64` or
  `ArrayBuffer` (bytes; a string literal is taken as UTF-8), or an import
  redeclares a runtime type with different fields
- a mount names an unknown instance, its path isn't clean and absolute (no
  `..`), or the same mount point appears twice in one view
- a startup request's path isn't clean, or its body names an unknown type or
  gets a field wrong (missing, extra, wrong type, or an integer out of range)
- an environment object is given mounts, or doesn't get exactly the config
  it needs, or a Wasm instance is given `send`

Errors carry `line:col` positions.

`--check` also prints each instance's view and the most it can ever reach.
Objects can only bind and expose what they can already see, so this is a
true upper bound:

```text
$ cargo run -q -- --check examples/static/static.img
net    sees nothing
files  sees nothing
cons   sees nothing
site   sees /files → files ro
       can reach at most: files ro
http   sees /net/tcp → net, /www → site, /dev/cons → cons
       can reach at most: net rw, files ro (via site), cons rw, site rw
```

## Environment objects

These are servers implemented by the runtime. Their config can only come
from the image, so no object can open another port or move a files root.

| Type | Config | Serves |
|---|---|---|
| `Environment.Console` | none | `watch /`: a stream of `Line` from stdin, ending at EOF. `write /`: prints the body |
| `Environment.Tcp` | `send Listen { address: "127.0.0.1:8080" }` | `read /address`: `Text`, the bound address. `watch /accept`: a stream of `Accepted { conn, peer }`. `watch /<conn>`: a stream of `Data` received, ending when the peer closes. `write /<conn>`: sends a `Data`'s bytes. `remove /<conn>`: closes our side |
| `Environment.Files` | `send FilesRoot { path: "./public" }` | `read`: `Data` with a MIME type from the extension, or `IsDirectory`. `stat`: `Info`. `list`: a stream of `Entry`, sorted. Read-only |

- **Console** prints a message whose only field is `text: string` verbatim,
  and anything else JSON-ish, e.g. `Stats {"lines": 1, "chars": 5, "average": 5.0}`.
  It reads stdin only if something is watching it by the time the image has
  started. With several Consoles, lines go to each in turn.
- **Tcp** binds when the image is loaded, so `/address` is right even for
  port 0. Data that arrives before anyone watches a connection is kept for
  the first watcher. Idle connections time out after 30s.
- **Files** resolves paths under the root (a relative root is relative to the
  image file). `..` never gets this far, and symlinks are resolved and must
  stay inside the root. Anything outside it is `NotFound`.

## The static HTTP server

The static example serves a small site: a landing page plus this README and
`docs/namespace.md`, rendered to HTML in the Othismo style by
`examples/static/site.mjs`. Run `make site` after editing either document;
the rendered pages are committed, so the example itself needs no build step.

```text
instance net   : Environment.Tcp     { send Listen    { address: "127.0.0.1:8080" } }
instance files : Environment.Files   { send FilesRoot { path: "./public" } }
instance cons  : Environment.Console

instance site : StaticSite {
  mount files /files readonly
}

instance http : HttpServer {
  mount net  /net/tcp
  mount site /www
  mount cons /dev/cons
  write /hosts/* Text { text: "/www" }
  write /hosts/files.localhost Text { text: "/www/static" }
}
```

- **`HttpServer`** is a bridge from HTTP to the namespace and knows nothing
  about files. It watches `/net/tcp/accept`, buffers each connection until a
  whole request has arrived, and turns it into a request at the host's root
  plus the URL path: GET is Read, HEAD is Stat, PUT is Write, DELETE is
  Remove and POST is Invoke. The answer becomes the response:
  - `Data` → 200 with its MIME type
  - `Info` → a HEAD response
  - `IsDirectory`, or a directory `Info`, for a path without a trailing `/` → 301
  - `NotFound` → 404
  - `ReadOnly` or `NotSupported` → 405
  - `Invalid` → 400

  Its own tree is `/hosts/<name>`. Writing a `Text` there serves that host
  from a path in its view, and `*` is the fallback. Malformed requests get
  400, oversized ones 413 or 431, and a `..` in the URL 400. Every response
  is logged to `/dev/cons`.
- **`StaticSite`** forwards each safe request to `/files`, adding
  `index.html` to paths ending in `/`, and refuses the rest. It never sees
  the files' contents: the answer goes straight back to http. Its tree also
  has `/hits`, a synthetic file counting what it forwarded, and `/static`,
  where it exposes `/files` read-only so those requests skip its code.
  `files.localhost` is rooted there.

Limitations: one request per connection (`Connection: close`), whole files
are held in memory, and socket writes happen on the delivery thread with a
10s timeout, so a stalled client can briefly hold up other connections.

## Writing an object

Instance type `Foo` runs `scripts/Foo.wasm` next to the image, compiled from
`scripts/Foo.ts`:

```ts
import { Line } from "real-talk/env"
import { Response, Server, Then, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "real-talk/ns"
import { Echoed } from "../messages"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }   // the host's entry points

class Echo extends Server {
  start(): void {
    ns.watch("/dev/cons", new OnLine())     // answers arrive at OnLine.answer
  }
  // serve(req: Request) handles requests made of our own tree; see docs/namespace.md.
}

/** Stands in for a closure: keep any state the answers need in fields. */
class OnLine extends Then {
  answer(res: Response): void {
    const body = res.body
    if (body instanceof Line) ns.write("/dev/cons", new Echoed((body as Line).text))
  }
}

run(new Echo())
```

Messages extend `Message`, override `typeName()` and `encode()`, and register
a decoder (see `examples/hello/messages.ts`):

```ts
export class Echoed extends Message {
  text: string
  constructor(text: string) { super(); this.text = text }
  typeName(): string { return "Echoed" }
  encode(w: Writer): void { w.string("text", this.text) }
}
registerMessage("Echoed", (r: Reader): Message => new Echoed(r.string("text")))
```

A body of a type the module didn't register arrives as an `Opaque` message.
It can't be read, but it can be passed on unchanged.

AssemblyScript has no union types, runtime reflection or capturing closures.
That is why bodies are tested with `instanceof`, why each message spells out
its own encoding, and why answers go to `Then` objects.

## Sandbox and runtime

- Each object gets its own Wasmtime `Store`. Its only imports are
  `real_talk.call`, `reply`, `forward`, `cancel`, `bind`, `unmount`,
  `expose` and `env.abort`, which AssemblyScript always emits. A module that
  imports anything else, such as WASI, `env.trace` or `env.seed` (which
  `Math.random` needs), is refused before it is instantiated.
- Every path is resolved in the sender's own view when the request is made.
  Paths with `..` are `Invalid`, and paths outside the view are `NotFound`.
  Unsafe verbs through a read-only mount get `ReadOnly` without reaching the
  server.
- Request ids are numbered for each server and tickets for each caller.
  Replying to an unknown request is an error, while replies to a cancelled
  request are dropped.
- Bodies are BSON documents. The first key is `$type` (the class name),
  followed by the fields (see `src/wire.rs` for the type mapping). Verbs,
  paths, statuses and tickets travel as arguments, not in the BSON.
- Delivery is synchronous and single-threaded through one FIFO queue. In
  order: views are built and module top-level code runs, then the image's
  startup requests are delivered, then every module's `start()` runs, then
  environment objects start listening. Stdin and every socket connection are
  read on their own threads, which post events to a channel, and the queue is
  drained completely after each event. The run ends when no event can arrive
  any more. If an instance aborts or traps, the run stops with an error
  naming that instance.
- Tests swap the environment's I/O for in-memory buffers through
  `runtime::Io` without changing the image.

## Where this differs from the v0.1 spec

- There are no ports, protocols or routes. Objects address each other by
  path, and the image only decides what each object can see. This replaced an
  earlier emit/reply model with routing trees, which needed new syntax for
  every new routing need.
- Startup messages are requests with a verb and a path. `send` is kept only
  for environment config, and anonymous messages are rejected.
- Object classes aren't `export`ed. asc warns that classes can't become Wasm
  exports, and the module's interface is the `rt_*` entry points instead.
