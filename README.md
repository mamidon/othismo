# real_talk

A runtime for **images**: declarative graphs of sandboxed AssemblyScript
objects that can only talk over the ports the image wires. If a capability
(stdin, stdout, …) isn't named in the image, no object can reach it.

## Quick start

Requires Rust and Node.

```sh
make hello            # npm install, asc → scripts/*.wasm, cargo run -- examples/hello/hello.img
make greeter          # a larger example
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
| `src/runtime.rs` | Wasmtime host, message loop, `Environment.Stdin` / `Environment.Stdout` |
| `src/render.rs` | How Stdout prints a message |
| `as/` | The `real-talk` AssemblyScript library that object modules import |
| `examples/` | `hello` (the acceptance image) and `greeter` |

## The image language

```text
import { Line, Echoed } from "./messages"   // resolved against messages.ts

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
  has a type other than `string`, `bool`, `i8`–`i64`, `u8`–`u64`, `f32` or `f64`
- a protocol or port uses a type that wasn't imported
- an instance's out port is unwired, wired twice, or doesn't exist
- a wiring target doesn't list every type the port may carry in its `in`
  (`Environment.Stdout` accepts anything)
- a `send` names an unknown type, gets a field wrong (missing, extra, wrong
  type, or an integer out of range), or is anonymous
- `Environment.Stdin` is used without importing a `Line` declared exactly as
  `{ text: string }`

Errors carry `line:col` positions.

## Writing an object

Each `protocol Foo` is implemented by `scripts/Foo.ts` next to the image and
compiled to `scripts/Foo.wasm`:

```ts
import { Message, OutPort, Receiver, bind, rt_inbox, rt_deliver } from "real-talk"
import { Line, Echoed } from "../messages"

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
- Delivery is synchronous through a single FIFO queue. Module start functions
  run first, then the startup `send`s (in image order), and then stdin is read
  one line at a time, with the queue drained after each line. The run ends at
  EOF once the queue is empty. If an instance aborts or traps, the run stops
  with an error naming that instance.
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
