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

```text
import { Line } from "real-talk/env"        // environment types (as/env.ts)
import { Echoed } from "./messages"         // resolved against messages.ts

protocol Echo {
  in Line                     // types the object accepts (comma-separated, repeatable)
  out out: Echoed             // named out port; may carry several types
}

instance stdin : Environment.Stdin {
  out -> echo                 // Stdin has one port, `out`, carrying Line
}
instance echo : Echo {
  send Greeting { prefix: "Hi", excited: true }   // typed startup message
  send Ping                                       // unit message
  out -> stdout, logger                           // fan-out
}
instance stdout : Environment.Stdout              // accepts any message
```

The checker rejects an image when:

- an imported name isn't an exported class in the referenced file, or a field
  has a type other than `string`, `bool`, `i8`–`i64`, `u8`–`u64`, `f32`, `f64`
  or `ArrayBuffer` (bytes; a string literal in a `send` is taken as UTF-8)
- a protocol or port uses a type that wasn't imported
- an instance's out port is unwired, wired twice, or doesn't exist
- a wiring target doesn't list every type the port may carry in its `in`
  (`Environment.Stdout` accepts anything)
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

| Protocol | In | Out ports | Config (startup, exactly once) |
|---|---|---|---|
| `Environment.Stdin` | none | `out: Line` | none |
| `Environment.Stdout` | anything | none | none (startup sends are printed) |
| `Environment.Socket` | `SocketWrite`, `SocketClose` | `out: SocketData, SocketClosed`, `events: Listening` | `send Listen { address: "127.0.0.1:8080" }` |
| `Environment.Files` | `ReadFile` | `out: FileContents, FileError` | `send FilesRoot { path: "./public" }` |

Only the image can supply config. The checker consumes those sends, and the
config types aren't in the protocols' `in` lists, so no object can rebind the
socket or move the files root. Anything the image grants is visible in the
image.

- **Socket** is a TCP listener. Each accepted connection gets a `conn` id.
  Incoming bytes arrive as `SocketData` and a peer close as `SocketClosed`.
  `SocketClose` half-closes our side once pending writes are flushed.
  `Listening` reports the bound address, which is useful with port 0. Idle
  connections time out after 30s.
- **Files** is read-only and resolves `ReadFile.path` under the root (a
  relative root is relative to the image file). Any `..` component is refused,
  and symlinks are resolved and must stay inside the root. Failures come back
  as `FileError` with `reason` set to `not_found`, `directory`, `forbidden` or
  `io`. `tag` is echoed so callers can match replies to requests.

## The static HTTP server

`examples/static/static.img` is a pipeline of four objects:

```
net (Environment.Socket) --SocketData--> http (HttpCodec) --HttpRequest--> server (StaticServer) --ReadFile--> files (Environment.Files)
         ^                                   |    ^                             |    ^                                 |
         +-------SocketWrite/SocketClose-----+    +---------HttpResponse--------+    +-----FileContents/FileError------+
```

- `HttpCodec` (AssemblyScript) buffers bytes per connection until a request is
  complete. It parses the request line, headers and a `Content-Length` body,
  and emits a typed `HttpRequest` with a percent-decoded `path`. Going the
  other way, it serializes `HttpResponse`. Malformed requests get 400, and
  oversized ones get 413 or 431, without reaching the server.
- `StaticServer` (AssemblyScript) has no file access of its own. It sends
  `ReadFile` to whatever the image wires `files` to and maps the reply to
  200, 301 (directory without a trailing `/`), 403, 404, 405 or 500. It sets
  `Content-Type` from the file extension and logs each request to stdout.

Limitations: one request per connection (`Connection: close`), whole files are
held in memory, and headers travel as a single `"name: value\r\n"` string
because message fields can't be maps or arrays (`HttpRequest.header(name)`
looks one up). Socket writes happen on the delivery thread with a 10s timeout,
so a stalled client can briefly hold up other connections.

## Writing an object

Each `protocol Foo` is implemented by `scripts/Foo.ts` next to the image and
compiled to `scripts/Foo.wasm`:

```ts
import { Message, OutPort, Receiver, bind, rt_inbox, rt_deliver } from "real-talk"
import { Line } from "real-talk/env"
import { Echoed } from "../messages"

export { rt_inbox, rt_deliver }      // the host's entry points into the module

class Echo extends Receiver {
  constructor(private out: OutPort<Echoed>) { super() }

  receive(msg: Message): void {
    if (msg instanceof Line) this.out.send(new Echoed((msg as Line).text))
  }
}

// One OutPort per `out` in the protocol, numbered in declaration order.
bind(new Echo(new OutPort<Echoed>(0)))
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
  `real_talk.send(port, ptr, len)` and `env.abort`, which AssemblyScript always
  emits. A module that imports anything else, such as WASI, `env.trace` or
  `env.seed` (which `Math.random` needs), is refused before it is instantiated.
- On every send the host checks that the port exists and is declared to carry
  the message's type. With `--check-shapes` it also checks the fields against
  the declaration.
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

- `Environment.Stdin` is wired explicitly (`out -> echo`), and the hello image
  is changed to match. It isn't special-cased, so every edge appears in the
  image.
- Anonymous `send { ... }` records are rejected. Every send names its type.
- Object classes aren't `export`ed. asc warns that classes can't become Wasm
  exports, and the module's interface is `rt_inbox` / `rt_deliver` instead.
- `receive(msg: Message)` with `instanceof`, rather than `receive(msg: Line)`,
  for the reason above.
