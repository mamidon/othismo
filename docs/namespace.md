# Namespaces

This document describes how objects in an image find and reach each other:
Layer 0 of [the protocol](protocol.md). Almost everything is done by
objects, by convention, in a Plan 9–style namespace; the runtime only
decides where each path leads.

## The model

Every instance has two trees.

- **Its view** is the set of paths it can send requests to. The image builds
  the initial view with `mount` and `bind`. The instance can rearrange its
  view at run time, but only using names it can already see.
- **Its served tree** is the set of paths it answers for. Nothing in it has to
  exist ahead of time. A served tree is whatever the instance's `serve` code
  says it is, like a Plan 9 file server.

Mounting instance `pages` at `/www` in `http`'s view means that when `http`
sends a request to `/www/index.html`, `pages` receives it as a request for
`/index.html` in its own served tree.

```text
http's view                         pages' view
  /net/tcp  → the listener            /files → files (read-only)
  /www      → pages
  /dev/cons → the console
```

An image has a view of its own as well. Its startup requests resolve in it,
its instances can be given parts of it with `bind`, and what it `export`s
comes from it (see [Building images](images.md)).

Views are private. The same path can mean different things to different
instances. Conventional names (`/dev/cons`, `/net/tcp`, `/files`) keep that
manageable.

### Names are the capabilities

A path is resolved in the sender's view. A path that isn't in the view is
`not_found`, exactly as if it didn't exist. Paths are plain strings, not
unforgeable tokens. Putting a path in a payload doesn't grant anything: the
receiver resolves it in its own view. Access is controlled by what each view
can see, and the images decide that.

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
path, on segment boundaries. `/www` matches `/www`, `/www/` and `/www/a/b`,
but not `/wwwx`. Then it checks the target's exposures (see below) for the
longest prefix of the remaining path, and repeats until it reaches a server.
A read-only mount or exposure anywhere along the way makes the result
read-only.

## Building views

An image builds each view from three lines (see
[Building images](images.md#views)):

- `mount <at> <instance> [readonly]` puts an instance's whole served tree at
  `<at>`.
- `bind <at> <path> [readonly]`, in an instance block, gives the instance
  what `<path>` means in the image's own view. That is how an image passes
  down what it was given, including the host's objects.
- `needs <at> <shape>`, at the top level, leaves `<at>` in the image's view to
  be filled by whoever instantiates it: a parent image, or for the root
  image, the runtime.

## Rearranging a view

An instance can change its own view, and splice parts of it into the tree
it serves, while it runs:

| Operation | Semantics |
|---|---|
| `bind(src, at, flags)` | `at` now means whatever `src` means in our view at this moment. |
| `unmount(at)` | `at` no longer means anything in our view. |
| `expose(at, src, flags)` | Requests under `at` in our served tree go straight to `src` in our view; they never reach our `serve`. |

The only flag so far is `READONLY`; `BEFORE` and `AFTER`, for union mounts,
are planned.

- **`bind` resolves `src` when it is called.** The new mount points at
  whatever `src` named at that moment. Unmounting `src` later doesn't affect
  it, as with Plan 9's `bind`.
- **`expose` is the delegation primitive.** It can only expose what the
  instance can see, so it never widens what is reachable.
- **Read-only only accumulates.** Something read-only stays read-only however
  many times it is bound or exposed again.

Every view is built by an image and none is inherited: there is no
equivalent of Plan 9's `rfork`, since objects don't make other objects.

```ts
import { Server, Request, Verb, Errno, Flag, ns, run } from "othismo"

class Site extends Server {
  start(): void {
    ns.expose("/static", "/files", Flag.ReadOnly)    // /static/** never reaches serve()
  }

  serve(req: Request): void {
    req.forward("/files" + (req.path.endsWith("/") ? req.path + "index.html" : req.path))
  }
}
run(new Site())
```

## Environment objects

Environment objects are servers implemented by the runtime, and they are the
only way to reach the outside world. No image declares one. An image says
what it needs with `needs`, and only the root image, the one being run, has
its needs filled by the runtime with the real thing:

```text
needs /net/tcp  Listener      # a TCP listener
needs /dev/cons Console       # stdin and stdout
needs /srv/www  Files         # a directory on the host
```

The trait a root need names picks the object. Each root need is an object
of its own: two `needs ... Listener` lines are two listeners. A root need
that names any other trait, or a shape with no trait's name, is an error
when the image is built. The root image can then configure each one, from
its own view, and give it to the instances that should have it with `bind`.

So everything a program can do to the host is listed in one place, the root
image's `needs`, and `--check` prints it.

An environment object is configured by an operator `write` of its root, which
only an image can send. No object can open another port or move a files
root. Changing the configuration once it has started isn't supported yet:
they answer later writes of their root with `not_supported`.

| Trait | Configured with | Serves |
|---|---|---|
| `Console` | nothing | `watch /`: a stream of `text/plain`, one line of stdin each, without its line ending, until EOF. `write /`: prints the payload on a line of its own: `text/*` as it is, a stat record as its fields, anything else as its type and length |
| `Listener` | `text/plain`: the address, e.g. `127.0.0.1:8080` | `read /address`: the bound address. `watch /accept`: a stream of `text/plain` connection names. `watch /{conn}/data`: the bytes received, as `application/octet-stream`, until the peer closes. `write /{conn}/data`: sends the payload's bytes. `read /{conn}/remote`: the peer's address. `remove /{conn}`: closes our side, once everything is sent |
| `Files` | `text/plain`: the directory, relative to the image | `read`: the file, with a content type from its extension. `stat` and `list`: stat records, the listing sorted by name. Read-only: unsafe verbs get `read_only` |

Console reads stdin only if something is watching it by the time the image
has started. When there are several Consoles, each line goes to the next one
in turn, and every watcher of that Console gets it.

A connection's data can have one watcher; a second gets `exists`. Data that
arrives before anyone watches is kept for the first watcher. Files answers
anything outside its root, including through symlinks, with `not_found`.

### In memory

The runtime also provides objects that serve the same traits and touch
nothing outside the image. They are ordinary objects, imported from `env`,
and any image can declare them and pass them to what it instantiates:

| Object | Serves |
|---|---|
| `MemoryConsole` | `Console`, and `Terminal`: `write /typed` types a line, `watch /printed` streams whatever is written to it |
| `MemoryListener` | `Listener`, and `Dialer`: `create /dial` with an `application/octet-stream` payload connects, the server receives the bytes, and the request's answers are everything the server writes, until it closes the connection |
| `PackedFiles` | `FileTree`, from files copied into the image when it was built (see [Building images](images.md#packed-files)) |

This is how an image is tested: the test image is a root with no needs, and
gives the image under test in-memory objects where its deployment would give
it real ones.

## Startup order

1. Every instance is created, and every view is built from the images'
   mounts and binds. Module top-level code runs. It may already make
   requests, which are queued.
2. Every image's startup requests are delivered, each image's in the order
   they are written, and an instantiated image's before those of the image
   that instantiates it, so it is set up by the time its parent configures
   it through what it exports.
3. `start()` runs on every Wasm instance, in start order.
4. Environment objects start listening (stdin, sockets).

**Start order** follows the mounts and binds: an instance starts after
everything it mounts, so whatever it finds in its view has been configured
and started first. Mounting an instance of an image counts as mounting what
it exports. Instances that don't depend on each other keep image order.
Where instances mount each other, the one that comes first in the image
starts last.

The run ends when the queue is empty and no outside event can arrive any more.

## What the checker can promise

An instance can only bind what it can see and only expose what it can see.
So the set of instances it can ever reach is bounded by the transitive closure
of the images' mount and bind edges, which can be computed without running
anything. Read-only flags carry through the closure, and an instance of an
image reaches only what it exports. `othismo --check` prints it:

```text
slot listen : text/plain = "127.0.0.1:8080" (default)

from the host:
  /net/tcp    listens on 127.0.0.1:8080
  /dev/cons   reads stdin, writes stdout

web.http    sees /net/tcp → /net/tcp, /dev/cons → /dev/cons, /www → web.pages
            can reach at most: /net/tcp rw, /dev/cons rw, web.pages rw, web.files ro (via web.pages)
web.pages   sees /files → web.files ro
            can reach at most: web.files ro
web.files   serves 12 built-in files, 152.5 KB
```

The checker also holds the images to their objects' interfaces (see
[Objects](objects.md)): every mount must serve what its object needs there,
and every startup request must be one its object serves.

## Not yet

- Union mounts (several servers at one path, tried in order, as Plan 9's
  `bind -b` and `bind -a`).
- A `/srv` convention for posting a served tree so that others can mount it.
- Timeouts on calls.
- Writable Files.
- Reconfiguring an environment object while it runs.
