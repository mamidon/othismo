# Namespaces

This document describes how objects in an image find and talk to each other.
It replaces the earlier model, in which objects `emit`ted typed messages and
the image routed them by `sender.Type` trees. That model needed new syntax for
every new routing need. Here, almost everything is done by objects, by
convention, in a Plan 9–style namespace.

## The model

Every instance has two trees.

- **Its view** is the set of paths it can send requests to. The image builds
  the initial view with `mount`. The instance can rearrange its view at run
  time, but only using names it can already see.
- **Its served tree** is the set of paths it answers for. Nothing in it has to
  exist ahead of time. A served tree is whatever the instance's `serve` code
  says it is, like a Plan 9 file server.

Mounting instance `site` at `/www` in `http`'s view means that when `http`
sends a request to `/www/index.html`, `site` receives it as a request for
`/index.html` in its own served tree.

```text
http's view                         site's view
  /net/tcp  → net                     /files → files (read-only)
  /www      → site
  /dev/cons → cons
```

The image has a view of its own as well, which only its startup requests use
(see [The image](#the-image)).

Views are private. The same path can mean different things to different
instances. Conventional names (`/dev/cons`, `/net/tcp`, `/files`) keep that
manageable.

### Names are the capabilities

A path is resolved in the sender's view. A path that isn't in the view is
`not_found`, exactly as if it didn't exist. Paths are plain strings, not
unforgeable tokens. Putting a path in a message doesn't grant anything: the
receiver resolves it in its own view. Access is controlled by what each view
can see, and the image decides that.

### Paths

Paths are absolute and `/`-separated. The runtime normalizes `//` and `.`
segments and keeps a trailing `/`, because `/docs` and `/docs/` can mean
different things to a server (HTTP certainly treats them differently). A path
containing a `..` segment, a NUL byte, or one that doesn't start with `/` is
rejected with `invalid` before it is resolved. Without that rule,
`root + clientPath` could step from `/www` into any other part of the sender's
view.

A trailing `/` means nothing in a mount point: `/www/` and `/www` name the
same one.

Resolution in a view picks the longest mount point that is a prefix of the
path, on segment boundaries. `/www` matches `/www`, `/www/` and
`/www/a/b`, but not `/wwwx`. Then it checks the target's exposures (see
below) for the longest prefix of the remaining path.

## Verbs

Every request carries a verb in its envelope, apart from its typed body. The
runtime and any intermediary can therefore tell whether a request changes
anything without understanding the body's type.

| Verb | Safe | Idempotent | Body | Reply |
|---|---|---|---|---|
| `read` | yes | yes | none | one message, conventionally `Data { mime, data }` |
| `stat` | yes | yes | none | `Info { kind, size, mime }` |
| `list` | yes | yes | none | a stream of `Entry { name, kind }` |
| `watch` | yes | yes | none | a stream, until it ends or is cancelled |
| `write` | no | yes | a message | ok |
| `create` | no | no | a message | `Entry` for what was made |
| `remove` | no | yes | none | ok |
| `invoke` | no | no | a message | anything: the escape hatch for RPC-shaped operations |
| `control` | no | no | a message or none | anything; only the operator may send it |

A **safe** verb doesn't change anything. A mount or exposure can be
**read-only**, in which case the runtime answers every unsafe verb sent
through it with `read_only` and the server never sees it. The restriction only
accumulates along a chain: something read-only stays read-only however many
times it is re-bound or re-exposed.

**`control`** is the operator's verb: the image sends it in its startup
requests, and objects can't. When an object calls with `control`, the runtime
answers `forbidden` without resolving the path. An object that accepts only
`control` at a path leaves that path to whoever runs the image; HttpServer's
`/hosts/<name>` is one.

Replies carry a **status**: `ok`, `not_found`, `read_only`, `not_supported`,
`is_directory`, `not_directory`, `exists`, `invalid`, `io` or `forbidden`. A
failure is just a reply with a status other than `ok`.

A request may get several replies (a **stream**): each one says whether it is
the last. `list` and `watch` use this, as does accepting connections. There
is no other streaming or subscription mechanism.

## The AssemblyScript SDK

Message classes, `Writer`, `Reader` and `registerMessage` stay in
`"othismo"`. The namespace SDK is `"othismo/ns"`. The conventional message
types are in `"othismo/env"`.

```ts
export enum Verb { Read, Stat, List, Watch, Write, Create, Remove, Invoke, Control }
export function isSafe(v: Verb): bool

export enum Status { Ok, NotFound, ReadOnly, NotSupported, IsDirectory, NotDirectory, Exists, Invalid, Io, Forbidden }
export enum Flag { None = 0, ReadOnly = 1 }

/** A request made of our served tree. Keep it to answer later if needed. */
export class Request {
  readonly verb: Verb
  readonly path: string            // relative to our root, e.g. "/index.html"
  readonly body: Message | null
  readonly cancelled: bool
  readonly finished: bool          // answered or forwarded; answering again aborts
  reply(body: Message | null): void          // Ok, and the last reply
  stream(body: Message): void                // Ok, more to come
  end(): void                                // Ok, no body, the last reply
  fail(status: Status): void                 // the last reply
  forward(path: string): void   // resolve `path` in OUR view; the answer goes straight to the caller
}

export class Response {
  readonly status: Status
  readonly body: Message | null
  readonly done: bool              // false while a stream is still open
  get ok(): bool
}

/** Stands in for a closure, which AssemblyScript lacks. Its fields hold the state. */
export class Then { answer(res: Response): void }

/** A request we made. */
export class Call { cancel(): void }

export namespace ns {
  // Requests. `then` may be null when no answer is wanted.
  function read(path: string, then: Then): Call
  function stat(path: string, then: Then): Call
  function list(path: string, then: Then): Call
  function watch(path: string, then: Then): Call
  function write(path: string, body: Message, then: Then | null): Call
  function create(path: string, body: Message, then: Then | null): Call
  function remove(path: string, then: Then | null): Call
  function invoke(path: string, body: Message, then: Then | null): Call
  function call(verb: Verb, path: string, body: Message | null, then: Then | null): Call

  // Rearranging our view. Synchronous; `source` must resolve in our view.
  function bind(source: string, at: string, flags?: Flag): Status
  function unmount(at: string): Status

  // Splices part of our view into our served tree at `at`. Requests under
  // `at` then go straight to `source`; they never reach our serve().
  function expose(at: string, source: string, flags?: Flag): Status

  /** `root` + `path`, or null if `path` is not a clean absolute path. */
  function join(root: string, path: string): string | null
}

export class Server {
  start(): void {}                                   // once the image is set up
  serve(req: Request): void { req.fail(Status.NotSupported) }
  cancel(req: Request): void {}    // the caller gave up on a stream; it is ended for us afterwards
}
export function run(server: Server): void
```

Some details of this API:

- **`Then` objects take the place of callbacks.** AssemblyScript closures
  can't capture variables, but classes have virtual methods, so a
  continuation object works as a closure. A connection's `OnData` object can
  *be* that connection's buffer. Underneath, the runtime numbers each call
  with a ticket, and the SDK maps tickets to `Then` objects.
- **`forward` makes proxies stateless.** A controller that maps paths onto
  another server never sees the answer.
- **`expose` is the delegation primitive.** It can only expose what the
  instance can see, so it never widens what is reachable.
- **Unknown types pass through.** A body of a type the module didn't
  register arrives as `Opaque`. It can't be read, but it can be passed on
  unchanged.
- **`bind` resolves `source` when it is called.** The new mount points at
  whatever `source` named at that moment. Unmounting `source` later doesn't
  affect it, as with Plan 9's `bind`.

### Example: a site controller

```ts
import { Message } from "othismo"
import { Server, Request, Verb, Status, Flag, ns, isSafe, run } from "othismo/ns"
import { Data } from "othismo/env"

class Site extends Server {
  hits: u64 = 0

  start(): void {
    ns.expose("/static", "/files", Flag.ReadOnly)    // /static/* never reaches serve()
  }

  serve(req: Request): void {
    if (req.path == "/hits") {                       // a synthetic file
      if (req.verb == Verb.Read) req.reply(new Data("text/plain", String.UTF8.encode(this.hits.toString())))
      else req.fail(Status.NotSupported)
      return
    }
    if (!isSafe(req.verb)) { req.fail(Status.ReadOnly); return }
    this.hits++
    req.forward("/files" + (req.path.endsWith("/") ? req.path + "index.html" : req.path))
  }
}
run(new Site())
```

## Environment objects

Environment objects are servers implemented by the runtime. They are the only
way to reach the outside world. An image declares one by its trait, from the
runtime's own header, `env`: `instance net : Listener`. The trait is all the
image and its objects know of it, and the runtime picks how to run it. Their
configuration is a `control` of their
root from the image (`control /net Listen { ... }`, with `net` mounted at
`/net` in the image's view), and objects can't send `control`, so no object can open another port
or move a files root. Changing it while the image runs isn't supported yet:
they answer `control` with `not_supported`.

| Trait | Config | Serves |
|---|---|---|
| `Console` | none | `watch /`: a stream of `Line`, one per line of stdin, ending at EOF. `write /`: prints the body |
| `Listener` | `Listen { address }` | `read /address`: `Text` with the bound address. `watch /accept`: a stream of `Accepted { conn }`. `watch /<conn>`: a stream of `Data` received, ending when the peer closes. `write /<conn>`: sends a `Data`'s bytes. `remove /<conn>`: closes our side |
| `Files` | `FilesRoot { path }` | `read`: `Data` with a MIME type from the extension. `stat`: `Info`. `list`: a stream of `Entry`, sorted by name. Read-only: unsafe verbs get `read_only` |

`Packed.Files` answers exactly as `Files` does, from files copied
into the image when it was built (see [Building images](images.md)). It isn't
an environment object, since it touches nothing outside the image.

Console reads stdin only if something is watching it by the time the image
has started. When there are several Console instances, each line goes to the
next one in turn, and every watcher of that Console gets it.

A Tcp connection can have one watcher; a second gets `exists`. Data that
arrives before anyone watches is kept for the first watcher. Files answers
anything outside its root, including through symlinks, with `not_found`. `Console` prints
a message whose only field is `text: string` verbatim, and anything else
JSON-ish.

## The image

What's left of the image language is instances, the mounts that make up each
view, and startup requests, plus the packing and slots
described in [Building images](images.md):

```text
require listen : Listen { address: "127.0.0.1:8080" }   # the host may change it

instance http : HttpServer {
  mount /net/tcp net
  mount /www site
  mount /dev/cons cons
}

instance site : StaticSite {           # scripts/StaticSite.wasm
  mount /files files readonly
}

instance net   : Listener
instance cons  : Console
instance files : Packed.Files { include "./public/" }

mount /net  net                        # the image's own view
mount /http http

control /net listen                                     # net's config
control /http/hosts/localhost Text { text: "/www" }     # a startup request to http
```

- `mount <path> <instance> [readonly]` puts that instance's whole served tree
  at `<path>` in a view. Inside an instance's block it builds that instance's
  view. At the top level it builds the image's own view. Instances can be
  declared in any order, so an image can start from the instance at the top
  of the namespace and work down.
- `<verb> <path> [Type { fields }]` is a request the image makes at startup,
  resolved in the image's view like any other request: `/http/hosts/localhost`
  reaches `/hosts/localhost` in `http`'s served tree because `http` is
  mounted at `/http`. The image can only send requests to what it mounts, so
  an image with no top-level mounts sends none. Requests can appear anywhere
  in the image, and are made in the order they are written. The image
  ignores replies, except that a failure is printed to stderr as a warning.
- `control` is the operator's verb, and these startup requests are the only
  place an image sends it. An environment object's config is a `control` of
  its root, so to configure one the image mounts it and sends that:
  `control /net listen`. Objects can't send `control` themselves.
- The runtime's message types (`Line`, `Text`, `Data`, `Info`, `Entry`,
  `Accepted`, `Listen`, `FilesRoot`) are always in scope; importing them
  `from env` is allowed but not needed. Any other type the image writes is
  imported from a header: `import { Greeting } from Greeter` takes it from
  that module, and `from "./shared.iface"` from a header file. An image never
  imports a type on its objects' behalf, only the ones it writes itself. A
  type is its name and its fields, so a header may redeclare one of the
  runtime's types only with exactly the same fields, and two headers that
  declare one name differently can't be used in one image.

### What the checker can promise

An instance can only bind what it can see and only expose what it can see.
So the set of instances it can ever reach is bounded by the transitive closure
of the image's mount edges, which can be computed without running anything.
Read-only flags carry through the closure. `othismo --check` prints it:

```text
slot listen : Listen { address: "127.0.0.1:8080" (default) }

the image sees /net → net, /http → http

http   sees /net/tcp → net, /www → site, /dev/cons → cons
       can reach at most: site rw, net rw, cons rw, files ro (via site)
site   sees /files → files ro
       can reach at most: files ro
net    listens on 127.0.0.1:8080
cons   reads stdin, writes stdout
files  serves 11 built-in files, 105.6 KB
```

The checker also holds the image to its objects' interfaces (see
[Objects](objects.md)): every mount must serve what its object needs there,
and every startup request must be one its object serves. While the image
runs, the runtime holds every object to its interface and every body to its
type; `--no-checks` turns that off.

## Startup order

1. Every instance is created and every view is built from the image's mounts.
   Module top-level code runs. It may already make requests, which are queued.
2. The image's startup requests are delivered, in the order they are written.
3. `start()` runs on every Wasm instance, in start order.
4. Environment objects start listening (stdin, sockets).

**Start order** follows the mounts: an instance starts after everything it
mounts, so whatever it finds in its view has been configured and started
first. Instances that don't depend on each other keep image order. Where
instances mount each other, the one that comes first in the image starts
last.

The run ends when the queue is empty and no outside event can arrive any more.

## The host ABI

The path, verb, status and ticket travel as arguments. Only bodies are BSON,
as before.

Imports (module `othismo`), plus `env.abort`:

| Import | Meaning |
|---|---|
| `call(verb, path, path_len, body, body_len, expect) -> ticket` | `body_len` 0 means no body. With `expect` 0, the ticket is 0 and replies are dropped |
| `reply(req, status, body, body_len, done)` | answer a request we are serving |
| `forward(req, path, path_len)` | hand a request to a path in our view |
| `cancel(ticket)` | give up on a call; later replies are dropped |
| `bind(src, src_len, at, at_len, flags) -> status` | |
| `unmount(at, at_len) -> status` | |
| `expose(at, at_len, src, src_len, flags) -> status` | |

Exports: `memory`, plus the following:

| Export | Meaning |
|---|---|
| `rt_inbox(len) -> ptr` | a buffer for the host to write into |
| `rt_serve(req, verb, path_len, body_len)` | the inbox holds the path, then the body |
| `rt_answer(ticket, status, body_len, done)` | the inbox holds the body |
| `rt_start()` | |
| `rt_cancel(req)` | optional. Without it, a cancelled request's replies are dropped until the module ends it |

Request ids are numbered per server and tickets per caller. Replying to an
unknown request id is an error that stops the run. Replies to a cancelled
request, and replies to a call made with `expect` 0, are dropped.

## Not yet

- Union mounts (several servers at one path, tried in order, as Plan 9's
  `bind -b` and `bind -a`).
- A `/srv` convention for posting a served tree so that others can mount it.
- Timeouts on calls.
- Writable Files.
