# Othismo

Othismo is a way of building programs out of parts you don't have to read.
Each part is a sandboxed object that can reach only what it has been given,
and a short, declarative image says who is given what. You review the image
and the interfaces; the runtime holds the code to them.

## Why

Agentic development promises a great deal of productivity, and to collect on
that promise you eventually have to stop reviewing every line an agent
writes. What works instead is treating the agent's code as a black box: you
pay attention to its interfaces with the outside world, specify what they
mean and what side effects they may have, and test against that.

That is a sound way to work, but today's languages do little to support it.
Boundaries between components are conventions. Any module can open a socket,
read a file anywhere on disk or reach into another module's state, and
nothing stops an agent from doing so. The only way to be confident it didn't
is to read the 20,000-line pull request carefully, which is the work you
were trying to stop doing.

Othismo makes the boundaries structural. The code you don't read runs in a
sandbox whose only way out is a set of names chosen by something small enough
to read. If the image doesn't give an object a name, the object can't reach
what it would refer to, however its code is written.

## Objects

An object is a WebAssembly module, today written in AssemblyScript, running
in its own sandbox. It is the black box: the part an agent writes and you
don't read.

It gets no ambient access to anything. There is no WASI, no clock, no file
system and no network; a module that imports any of them is refused before
it runs. What an object can do is send requests to paths in its namespace and
answer requests made of its own. Everything else about it is its own
business.

Objects are single-threaded and share nothing. They react to requests and to
the answers to their own requests, and that is their whole life.

What you read instead of the code is the object's **interface**, a header
that travels inside the module. It declares the paths the object serves and
what each takes and answers, what it needs to find in its view, and its
documentation. The checker holds every image to its objects' interfaces, and
the runtime holds every object to its own while it runs. [Objects](objects.md)
covers them in detail.

## Requests

A request is a path, a verb and, for some verbs, a payload: a content type
and some bytes. There are seven verbs. `read`, `stat`, `list` and `watch` ask
about something without changing it; `write`, `create` and `remove` may
change it. Like HTTP's methods, nothing makes a server honour that. A server
that changes something when it handles a `read` is simply wrong, as a GET
that deletes a record is.

A failure is just an answer with an error instead of `ok`, and a stream is
just several answers to one request. There are no exceptions crossing
objects, no callbacks and no separate subscription mechanism.

The runtime never looks inside a payload. Its content type, such as
`text/html` or `application/bson`, says which format its bytes are in, and
what they mean is between the objects that exchange them, as an HTTP body
is between a client and a server. Othismo's own part, the namespace and the
protocol, stays small and fixed however many kinds of payload there are.
[The protocol](protocol.md) sets out all three layers and how they compare
with 9P.

## Namespaces

Every object has its own namespace, as every process does in Plan 9. It
has two sides: its **view**, the paths it can send requests to, and its
**served tree**, the paths it answers for. Mounting one object into
another's view at `/www` means that when the second sends a request to
`/www/index.html`, the first receives a request for `/index.html`.

What isn't in an object's view doesn't exist for it. A path it can't see is
`not_found`, exactly as if nothing were there, and mentioning a path in a
payload gives the receiver nothing, because the receiver resolves it in its
own view. An object can rearrange its view and share parts of it while it
runs, but only using what it can already see, and a read-only mount stays
read-only however it is passed on.

Most structure is built by objects following conventions, not by new
features of the runtime. The console is at `/dev/cons`, the network at
`/net/tcp`. An HTTP server is an object that turns HTTP requests into
namespace requests and knows nothing about files; a static site is an object
that forwards reads to a files tree it was given read-only, and never sees
the bytes it serves.

## The edge

The only way any object reaches the world outside the image is through
**environment objects**, built into the runtime: the console, TCP and the
host's files. No image declares one. An image says what it **needs**, and
only the image being run has its needs filled with the real thing; the
images it uses get them passed down, or get stand-ins that run in memory.

So the question "what can this program do to the machine it runs on" has a
short answer, written in one place: whatever the needs of the image being
run allow, and nothing else. Their configuration comes from that image
alone, so no object can open another port or reach files that weren't given
to the image.

## Images

An image is a file that declares objects, what each one can see, and
optionally some requests to configure them when it starts. Images and
interfaces are written in the same language, `.oth`, and an image can use
another as an object of its own: one made of objects, serving only what it
exports.

This one serves this site. It touches nothing on the host itself; it needs
a listener and a console:

```text
needs /net/tcp  Listener
needs /dev/cons Console

instance http : HttpServer {
  bind  /net/tcp  /net/tcp
  bind  /dev/cons /dev/cons
  mount /www pages
}

instance pages : StaticSite {
  mount /files files readonly
}

# brings the host's ./public folder into the image, at build time
instance files : PackedFiles { include "./public/" }

# tells http to serve every host from /www in its view
mount /http http
write /http/hosts/* "/www"
```

And this one runs it, giving it the real thing:

```text
import Site from "./site.oth"

# a parameter, which whoever runs the image may change
require listen : text/plain = "127.0.0.1:8080"

needs /net/tcp  Listener
needs /dev/cons Console

instance web : Site {
  bind /net/tcp  /net/tcp
  bind /dev/cons /dev/cons
}

# configures the socket to listen where `listen` says
write /net/tcp listen
```

Because an object can only pass on what it can see, the most any object can
ever reach is fixed by the images' mounts and can be worked out without
running anything. `othismo --check` prints it, along with everything the
image touches on the host:

```text
slot listen : text/plain = "127.0.0.1:8080" (default)

from the host:
  /net/tcp    listens on 127.0.0.1:8080
  /dev/cons   reads stdin, writes stdout

web.http    sees /net/tcp → /net/tcp, /dev/cons → /dev/cons, /www → web.pages
            can reach at most: /net/tcp rw, /dev/cons rw, web.pages rw, web.files ro (via web.pages)
web.pages   sees /files → web.files ro
            can reach at most: web.files ro
web.files   serves 12 built-in files, 150.0 KB
```

That is a promise about the code without reading the code: `StaticSite`
can't write anything, can't reach the network and can't see any file outside
`public/`, whatever it was written to do.

The site's tests are an image too. It gives the site an in-memory listener
instead of a real one, and puts a test suite beside it to dial it.

## Building and deploying

An image can be built, as a Docker image is built from a Dockerfile, into a
single file holding its compiled objects, their interfaces and any files it
packs. The built file is what you deploy, and it is the same thing that was
checked: nothing outside it is read when it runs, except through its needs.

What changes between deployments is declared as a **slot**: a value the
image requires from whoever runs it, such as the address to listen on. The
image's author decides the slot's type and defaults; the deployer supplies
the rest. Slots can stand in for payloads but never change mounts, so a
deployer can't widen what any object reaches, and `--check` on the built
file still tells the whole story.

## Where it comes from

- **Plan 9**: per-process namespaces, mounts, a small fixed protocol, and
  servers whose trees are whatever their code says they are. Most of
  Othismo's model is Plan 9's, applied to objects in one program instead of
  processes on a network.
- **HTTP**: verbs that say what handling a request implies, and bodies whose
  content type says what they are.
- **Docker**: an image built from a source file into one artifact that runs
  anywhere, configured from outside by the people who deploy it.
- **Smalltalk**, to a lesser extent: a system made of objects that interact
  only by sending each other messages, kept and shipped as an image.

## Where it stands

Othismo is a work in progress, and the runtime here, `othismo`, is a
from-scratch rewrite that started from the top: the model first, with the
semantics adjusted as they are used.

What it has so far: the namespace and the protocol, payloads the runtime
never reads, with BSON as their first format, one `.oth` language for
images and interfaces, images used as objects, and environment objects
given only as the needs of the image being run.

Some things the premise needs aren't there yet. A trapped object stops the
whole run, and calls never time out.

The details are in [The protocol](protocol.md), [Namespaces](namespace.md),
[Objects](objects.md) and [Building images](images.md).
