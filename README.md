# real_talk

A runtime for **images**: declarative graphs of sandboxed AssemblyScript
objects that can only talk over the ports the image wires. If a capability
(stdin, stdout, …) isn't named in the image, no object can reach it.

## Quick start

Requires Rust and Node.

```sh
make hello            # npm install, asc → scripts/*.wasm, cargo run -- examples/hello/hello.img
make greeter          # a larger example
make static           # a static HTTP server on http://127.0.0.1:8080
make test             # build the wasm, then cargo test
```

After `make wasm`, run any image directly:

```sh
cargo run -- examples/hello/hello.img
cargo run -- --check image.img          # validate only
cargo run -- --check-shapes image.img   # also check every message's fields in transit
```

## Layout

| Path | What |
|---|---|
| `src/image.rs` | Lexer and parser for `.img` files |
| `src/messages.rs` | Minimal reader for `export class X { field: type }` in AssemblyScript |
| `src/check.rs` | Resolves imports and protocols and validates wiring and startup sends → `Program` |
| `src/wire.rs` | BSON wire format |
| `src/runtime.rs` | Wasmtime host and message loop |
| `src/env.rs` | Environmental objects: Stdin, Stdout, Socket, Files |
| `src/render.rs` | How Stdout prints a message |
| `as/index.ts` | The `real-talk` AssemblyScript library that object modules import |
| `as/env.ts` | Environment message types (`"real-talk/env"`) |
| `examples/` | `hello` (the acceptance image), `greeter`, `static` (HTTP server) |

## The image language

Objects never know who they talk to. An object can do two things: **emit** a
message, which goes wherever the image routes it, or **reply** to whoever sent
the message it's handling. The image declares what each protocol accepts,
emits and replies with, which instances exist, and the routes.

```text
import { Line } from "real-talk/env"        // environment types (as/env.ts)
import { Echoed, Greeting } from "./messages"

protocol Echo {
  in Line, Greeting           // types it accepts, routed or replied to it
  emits Echoed                // types it may emit; the image routes them
  replies Ping                // types it may reply with (optional)
}

instance stdin  : Environment.Stdin
instance echo   : Echo {
  send Greeting { prefix: "Hi", excited: true }   // typed startup message
  send Ping                                       // unit message
}
instance stdout : Environment.Stdout              // accepts any message

// Routes: `sender.Type { targets }`. A target is an instance, or a nested
// route for what that instance emits in turn.
stdin.Line {
  echo.Echoed { stdout, logger }                  // fan-out: both get a copy
}
```

Routing is global. Nesting is only a readable way to write the edges, so
`echo.Echoed { ... }` applies to every `Echoed` that echo emits, however it was
triggered. Each `sender.Type` is routed in exactly one place, and
`sender.*` covers every type the sender emits. A type that isn't routed is
dropped; `--check` lists such types as notes. Replies need no routes: they go
back along the route the request came in on.

The checker rejects an image when:

- an imported name isn't an exported class in the referenced file, or a field
  has a type other than `string`, `bool`, `i8`–`i64`, `u8`–`u64`, `f32`, `f64`
  or `ArrayBuffer` (bytes; a string literal in a `send` is taken as UTF-8)
- a protocol uses a type that wasn't imported
- a route names an unknown instance, a type the sender doesn't emit, or a
  target that doesn't accept it (`Environment.Stdout` accepts anything)
- a route's target replies with a type the sender doesn't accept
- the same `sender.Type` is routed twice, including through `sender.*`
- a `send` names an unknown type, gets a field wrong (missing, extra, wrong
  type, or an integer out of range), or is anonymous
- an environment protocol is used without importing every message type it
  handles, or one of those types differs from its declaration in `as/env.ts`
- `Environment.Socket` or `Environment.Files` doesn't have exactly one config
  send (see below)

Errors carry `line:col` positions.

## Environmental objects

These are the only way an image reaches the outside world. Their message types
are declared in `as/env.ts` and imported from `"real-talk/env"`.

| Protocol | In | Emits | Replies | Config (startup, exactly once) |
|---|---|---|---|---|
| `Environment.Stdin` | none | `Line` | none | none |
| `Environment.Stdout` | anything | none | none | none (startup sends are printed) |
| `Environment.Socket` | `SocketWrite`, `SocketClose` | `SocketData`, `SocketClosed`, `Listening` | none | `send Listen { address: "127.0.0.1:8080" }` |
| `Environment.Files` | `ReadFile` | none | `FileContents`, `FileError` | `send FilesRoot { path: "./public" }` |

Only the image can supply config. The checker consumes those sends, and the
config types aren't in the protocols' `in` lists, so no object can rebind the
socket or move the files root. Anything the image grants is visible in the
image.

- **Socket** is a TCP listener. Each accepted connection gets a `conn` id.
  Incoming bytes are emitted as `SocketData` and a peer close as
  `SocketClosed`. Whoever receives them answers with `SocketWrite` and
  `SocketClose` replies. `SocketClose` half-closes our side once pending
  writes are flushed.
  `Listening` reports the bound address, which is useful with port 0. Idle
  connections time out after 30s.
- **Files** is read-only and resolves `ReadFile.path` under the root (a
  relative root is relative to the image file). Any `..` component is refused,
  and symlinks are resolved and must stay inside the root. Failures come back
  as `FileError` with `reason` set to `not_found`, `directory`, `forbidden` or
  `io`. Every `ReadFile` gets a reply, which echoes `tag` so the caller can
  match it to its request.

## The static HTTP server

`examples/static/static.img` routes requests down a tree. Every response is a
reply that retraces the path back up:

```text
net.SocketData {
  http.HttpRequest {
    server.ReadFile { files }
    server.Line     { stdout }
  }
}
net.SocketClosed { http }
net.Listening    { stdout }
```

That is, files replies to server with `FileContents`, server replies to http
with `HttpResponse`, and http replies to net with `SocketWrite`.
`net.*` isn't used because `Listening` goes to stdout and not to http.

- `HttpCodec` (AssemblyScript) buffers bytes per connection until a request is
  complete. It parses the request line, headers and a `Content-Length` body,
  and emits a typed `HttpRequest` with a percent-decoded `path`. It keeps the
  socket's reply handle for each connection, and when an `HttpResponse`
  arrives it serializes the response and replies with it to that socket.
  Malformed requests get 400, and oversized ones get 413 or 431, without
  reaching the server.
- `StaticServer` (AssemblyScript) has no file access of its own. It keeps the
  reply handle for each request and emits `ReadFile`. The image routes that
  to `files`, and the reply decides the response: 200, 301 (directory without
  a trailing `/`), 403, 404, 405 or 500. It sets `Content-Type` from the file
  extension and emits a `Line` per request for logging.

Limitations: one request per connection (`Connection: close`), whole files are
held in memory, and headers travel as a single `"name: value\r\n"` string
because message fields can't be maps or arrays (`HttpRequest.header(name)`
looks one up). Socket writes happen on the delivery thread with a 10s timeout,
so a stalled client can briefly hold up other connections.

## Writing an object

Each `protocol Foo` is implemented by `scripts/Foo.ts` next to the image and
compiled to `scripts/Foo.wasm`:

```ts
import { Message, Receiver, Sender, bind, emit, rt_inbox, rt_deliver } from "real-talk"
import { Line } from "real-talk/env"
import { Echoed } from "../messages"

export { rt_inbox, rt_deliver }      // the host's entry points into the module

class Echo extends Receiver {
  receive(msg: Message, from: Sender): void {
    if (msg instanceof Line) emit(new Echoed((msg as Line).text))
    // or: from.reply(...) to answer the sender. `from` may be kept and
    // used later, e.g. once some other request it triggered has come back.
  }
}

bind(new Echo())
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

AssemblyScript has no union types or runtime reflection. That is why
`receive` takes a `Message` and branches on `instanceof`, and why each message
spells out its own encoding.

## Sandbox and runtime

- Each object gets its own Wasmtime `Store`. Its only imports are
  `real_talk.emit(ptr, len)`, `real_talk.reply(sender, ptr, len)` and
  `env.abort`, which AssemblyScript always emits. A module that imports
  anything else, such as WASI, `env.trace` or `env.seed` (which `Math.random`
  needs), is refused before it is instantiated.
- `rt_deliver(sender)` passes a reply handle. Handles are numbered separately
  for each receiver and only ever name instances that have messaged it, so a
  made-up handle can't reach anything new. An unknown handle is an error, and
  handle 0 means the image itself (startup sends), whose replies are dropped.
- On every emit or reply the host checks that the protocol declares that
  type. A reply's recipient must also accept the type. With `--check-shapes`
  the host also checks the fields against the declaration.
- Messages are BSON documents. The first key is `$type` (the class name),
  followed by the fields (see `src/wire.rs` for the type mapping).
- Delivery is synchronous and single-threaded through one FIFO queue. Module
  start functions run first, then the startup `send`s (in image order). After
  that, environment objects start: stdin and every socket connection are read
  on their own threads, which post events to a channel. The queue is drained
  completely after each event. The run ends when no event can arrive any more
  (stdin hit EOF and nothing is listening). If an instance aborts or traps,
  the run stops with an error naming that instance.
- `Environment.Stdout` prints a message whose only field is `text: string`
  verbatim. Anything else prints JSON-ish, e.g.
  `Stats {"lines": 1, "chars": 5, "average": 5.0}`.
- Tests swap the environment's I/O for in-memory buffers through
  `runtime::Io` without changing the image.

## Where this differs from the v0.1 spec

- There are no out ports. Objects `emit` or `reply`, and the image routes by
  `sender.Type` (see above), so objects don't take ports in their
  constructors. `Environment.Stdin` is routed explicitly
  (`stdin.Line { echo... }`) rather than special-cased.
- Anonymous `send { ... }` records are rejected. Every send names its type.
- Object classes aren't `export`ed. asc warns that classes can't become Wasm
  exports, and the module's interface is `rt_inbox` / `rt_deliver` instead.
- `receive(msg: Message, from: Sender)` with `instanceof`, rather than
  `receive(msg: Line)`, for the reason above.
