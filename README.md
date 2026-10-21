# real_talk

A runtime for **images**: declarative sets of sandboxed AssemblyScript
objects that reach each other only through paths in their own namespaces. If
the image doesn't put something in an object's view, the object can't reach
it. Environment objects (console, TCP, files) are the only way out to the
world. An image can be built into one file that holds its modules and files
and runs anywhere.

The model is described in [docs/namespace.md](docs/namespace.md), and built
images in [docs/images.md](docs/images.md). This README covers using them.

## Quick start

Requires Rust and Node. The first `make` runs `npm install` and compiles each
example's `scripts/*.ts` to `scripts/*.wasm` with the AssemblyScript compiler.

```sh
make hello            # run examples/hello: echoes each line you type (Ctrl-D to end)
make greeter          # run examples/greeter: greets each name you type
make static           # run examples/static: serves these docs on http://127.0.0.1:8080
make static-built     # build examples/static/static.rti, then run that
make test             # compile the wasm, then cargo test
```

## Running the examples

Every example is an image, a `.img` file next to its `scripts/`. After
`make wasm`, run one from source with `cargo run -- <image>`. Running a
`.img` builds it in memory first, so nothing is written to disk.

To deploy an image, build it into one `.rti` file and run that instead. The
file holds the compiled modules, any packed files and the parsed message
types, so it runs anywhere `real_talk` does, with nothing else beside it.

### hello

Echo watches the console and writes back each line.

```sh
cargo run -- examples/hello/hello.img                     # type lines; Ctrl-D ends the run
printf 'hello\nworld\n' | cargo run -- examples/hello/hello.img

cargo run -- build examples/hello/hello.img -o hello.rti
printf 'hello\n' | cargo run -- hello.rti
```

### greeter

Greeter is configured by a startup request. It greets each name, keeps
running statistics, and has a second object shout the greeting.

```sh
printf 'world\n' | cargo run -- examples/greeter/greeter.img
# Hello, world!
# Stats {"lines": 1, "chars": 5, "average": 5.0}
# HELLO, WORLD!

cargo run -- build examples/greeter/greeter.img -o greeter.rti
printf 'Ada\n' | cargo run -- greeter.rti
```

### static

An HTTP server for a small site: these docs, rendered. The site is packed
into the image when it is built, and the address to listen on is a slot with
`127.0.0.1:8080` as its default. Stop it with Ctrl-C.

```sh
cargo run -- examples/static/static.img                   # http://127.0.0.1:8080
cargo run -- examples/static/static.img --set listen.address=127.0.0.1:9000

cargo run -- build examples/static/static.img -o static.rti
cargo run -- static.rti --set listen.address=0.0.0.0:8080 # e.g. on the machine you deploy to
```

`make static-built` builds `examples/static/static.rti` (rebuilding it when
the image, a module or a page changes) and runs it on the default address.
Run `make site` after editing this README or the docs, so the site serves the
new text.

### Inspecting an image

These work on a `.img` and a `.rti` alike:

```sh
cargo run -- --check static.rti                 # slots, what it touches on the host, views and reach
cargo run -- --check-shapes examples/hello/hello.img   # run, checking every body's fields in transit
cargo run -- unpack static.rti out/             # out/modules, out/files/<instance>, out/sources
```

Slot values can also come from a file, applied in order before any `--set`:

```sh
printf 'listen { address: "0.0.0.0:8080" }\n' > prod.vals
cargo run -- static.rti --with prod.vals
```

`build -v` lists the packed paths a later `include` replaced, and
`--no-sources` leaves out the reference copies of the image's sources.
`run` before the image is optional: `cargo run -- run static.rti` is the same.

## Layout

| Path | What |
|---|---|
| `src/image.rs` | Lexer and parser for `.img` files |
| `src/messages.rs` | Minimal reader for `export class X { field: type }` in AssemblyScript |
| `src/check.rs` | Resolves imports, slots, mounts, config and startup requests → `Program` |
| `src/pack.rs` | A built image: building, sealing to and opening `.rti` files, unpacking |
| `src/tree.rs` | Packed file trees and the `include` rules |
| `src/slots.rs` | Filling `require` slots from defaults, value files and `--set` |
| `src/ns.rs` | Paths, verbs, statuses, views, exposures and the static reach analysis |
| `src/wire.rs` | BSON wire format |
| `src/runtime.rs` | Wasmtime host, the host ABI and the message loop |
| `src/env.rs` | Environment objects: Console, Tcp, Files, and Packed.Files |
| `src/render.rs` | How Console prints a message |
| `as/index.ts` | `"real-talk"`: `Message`, `Writer`, `Reader`, `registerMessage` |
| `as/ns.ts` | `"real-talk/ns"`: the namespace SDK |
| `as/env.ts` | `"real-talk/env"`: the runtime's message types |
| `examples/` | `hello` (the acceptance image), `greeter`, `static` (HTTP server) |
| `examples/static/site.mjs` | Renders this README and the docs into the static example's site (`make site`) |
| `docs/namespace.md` | The design: model, verbs, SDK, ABI |
| `docs/images.md` | Built images: the file format, packed files and slots |

## The image language

An image declares instances. Each instance block holds that instance's view
(`mount`), environment config (`send`), files to pack (`include`), and
requests the image makes of it at startup (a verb and a path). Slots
(`require`) are messages the host fills in when the image runs.

```text
import { Greeting, Stats } from "./messages"   // types the image or its objects use

require greeting : Greeting { prefix: "Hello, " }   // excited must come from the host

instance cons : Environment.Console

instance shouter : Shouter {                   // runs scripts/Shouter.wasm
  mount cons /dev/cons
}

instance greeter : Greeter {
  mount cons /dev/cons                         // greeter's view: two mounts
  mount shouter /shouter
  write /greeting greeting                     // a slot stands in for a message
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
- `include "<path>" [at /path]` packs a file, or everything under a
  directory when the path ends in `/`, into a `Packed.Files` instance when
  the image is built. See below.
- `require <name> : Type [{ defaults }]` declares a slot. Its name can stand
  wherever a message can, in `send` or as a startup request's body. Fields in
  the block are defaults; the rest must come from the host:
  `--with <file>` (`greeting { excited: true }`, in order), then
  `--set greeting.excited=true`, a later value winning field by field. The
  image won't start until every field has a value, and `--check` shows each
  value and where it came from. Slots can't change mounts, so views are fixed
  when the image is built.
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
- a slot reuses a message type's name or another slot's, or its defaults get
  a field wrong; or a slot is used with fields of its own
- `include` appears outside a `Packed.Files` instance

Errors carry `line:col` positions.

`--check` also prints each slot's value, what each environment object touches
on the host, and each instance's view and the most it can ever reach. Objects
can only bind and expose what they can already see, so this is a true upper
bound:

```text
$ cargo run -q -- --check examples/static/static.img
slot listen : Listen { address: "127.0.0.1:8080" (default) }

net    listens on 127.0.0.1:8080
cons   reads stdin, writes stdout
files  serves 9 built-in files, 88.7 KB
site   sees /files → files ro
       can reach at most: files ro
http   sees /net/tcp → net, /www → site, /dev/cons → cons
       can reach at most: net rw, cons rw, files ro (via site), site rw
```

## Environment objects

These are servers implemented by the runtime. Their config can only come
from the image, so no object can open another port or move a files root.

| Type | Config | Serves |
|---|---|---|
| `Environment.Console` | none | `watch /`: a stream of `Line` from stdin, ending at EOF. `write /`: prints the body |
| `Environment.Tcp` | `send Listen { address: "127.0.0.1:8080" }` | `read /address`: `Text`, the bound address. `watch /accept`: a stream of `Accepted { conn, peer }`. `watch /<conn>`: a stream of `Data` received, ending when the peer closes. `write /<conn>`: sends a `Data`'s bytes. `remove /<conn>`: closes our side |
| `Environment.Files` | `send FilesRoot { path: "./public" }` | `read`: `Data` with a MIME type from the extension, or `IsDirectory`. `stat`: `Info`. `list`: a stream of `Entry`, sorted. Read-only |
| `Packed.Files` | `include` | The same as `Environment.Files`, from files packed into the image when it was built. Touches nothing on the host |

- **Console** prints a message whose only field is `text: string` verbatim,
  and anything else JSON-ish, e.g. `Stats {"lines": 1, "chars": 5, "average": 5.0}`.
  It reads stdin only if something is watching it by the time the image has
  started. With several Consoles, lines go to each in turn.
- **Tcp** binds when the image is loaded, so `/address` is right even for
  port 0. Data that arrives before anyone watches a connection is kept for
  the first watcher. Idle connections time out after 30s.
- **Files** resolves paths under the root (a relative root is relative to the
  directory holding the image, `.img` or `.rti`). `..` never gets this far,
  and symlinks are resolved and must stay inside the root. Anything outside it
  is `NotFound`.
- **Packed.Files** is a snapshot. `include "dir/"` (or `"dir/*"`) takes
  everything under `dir`, hidden files included, and `include "a.txt"` takes
  one file. Paths are relative to the image, and symlinks are followed
  wherever they point. A directory's contents land at the root and a file at
  `/<name>`, unless `at /path` says otherwise. Later includes win:
  directories merge, and a file and a directory replace each other.

## The static HTTP server

The static example serves a small site: a landing page plus this README,
`docs/namespace.md` and `docs/images.md`, rendered to HTML in the Othismo style by
`examples/static/site.mjs`. Run `make site` after editing any of them;
the rendered pages are committed, so the example itself needs no build step.
The pages are packed into the image, so `static.rti` is the whole site.

```text
require listen : Listen { address: "127.0.0.1:8080" }

instance net   : Environment.Tcp     { send listen }
instance cons  : Environment.Console
instance files : Packed.Files        { include "./public/" }

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
