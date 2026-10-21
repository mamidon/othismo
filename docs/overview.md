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

## Messages

A request is a verb, a path and, for some verbs, a typed message as its body.
The message types are declared alongside the objects and are the interfaces
you specify: what an object accepts, what it answers, and what each field
means.

The verb travels outside the body, and that matters more than it looks. The
verbs are modelled on HTTP's: `read`, `stat`, `list` and `watch` are safe and
change nothing, while `write`, `create`, `remove` and `invoke` may. Because
the runtime sees the verb without having to understand the body, it can hold
an object to "read only" whatever its code tries, and so can any object
standing in between.

A failure is just an answer with a status other than `Ok`, and a stream is
just several answers to one request. There are no exceptions crossing
objects, no callbacks and no separate subscription mechanism.

## Namespaces

Every object has its own namespace, as every process does in Plan 9. It
has two sides: its **view**, the paths it can send requests to, and its
**served tree**, the paths it answers for. Mounting one object into
another's view at `/www` means that when the second sends a request to
`/www/index.html`, the first receives a request for `/index.html`.

What isn't in an object's view doesn't exist for it. A path it can't see is
`NotFound`, exactly as if nothing were there, and mentioning a path in a
message gives the receiver nothing, because the receiver resolves it in its
own view. An object can rearrange its view and share parts of it while it
runs, but only using what it can already see, and a read-only mount stays
read-only however it is passed on.

Most structure is built by objects following conventions, not by new
features of the runtime. The console is at `/dev/cons`, the network at
`/net/tcp`. An HTTP server is an object that turns HTTP requests into
namespace requests and knows nothing about files; a static site is an object
that forwards reads to a files tree it was given read-only, and never sees
the bytes it serves. An earlier design routed typed messages between objects
with its own routing syntax, and every new kind of routing needed more of
it. Namespaces made that syntax unnecessary.

## The edge

Some objects are built into the runtime. **Environment objects** are the
console, TCP and the host's files, and they are the only way any object
reaches the world outside the image. Their configuration comes from the image
alone, so no object can open another port or move a files root.

These are the side effects you specify. The question "what can this program
do to the machine it runs on" has a short answer: whatever its environment
objects allow, and nothing else.

## Images

An image is a file that declares the objects, what each one's view contains,
and the requests made of each at startup. It is the part you review, and it
is meant to be short enough that you can. This one, slightly trimmed, serves
this site:

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
}
```

Because an object can only pass on what it can see, the most any object can
ever reach is fixed by the image's mounts and can be worked out without
running anything. `real_talk --check` prints it, along with everything the
image touches on the host:

```text
slot listen : Listen { address: "127.0.0.1:8080" (default) }

net    listens on 127.0.0.1:8080
cons   reads stdin, writes stdout
files  serves 9 built-in files, 75.5 KB
site   sees /files → files ro
       can reach at most: files ro
http   sees /net/tcp → net, /www → site, /dev/cons → cons
       can reach at most: net rw, cons rw, files ro (via site), site rw
```

That is a promise about the code without reading the code: `StaticSite`
can't write anything, can't reach the network and can't see any file outside
`public/`, whatever it was written to do.

## Building and deploying

An image can be built, as a Docker image is built from a Dockerfile, into a
single file holding its compiled objects, its message types and any files it
packs. The built file is what you deploy, and it is the same thing that was
checked: nothing outside it is read when it runs, except through its
environment objects.

What changes between deployments is declared as a **slot**: a message the
image requires from whoever runs it, such as the address to listen on. The
image's author decides the slot's type and defaults; the deployer supplies
the rest. Slots can stand in for messages but never change mounts, so a
deployer can't widen what any object reaches, and `--check` on the built
file still tells the whole story.

## Where it comes from

- **Plan 9**: per-process namespaces, mounts, and servers whose trees are
  whatever their code says they are. Most of Othismo's model is Plan 9's,
  applied to objects in one program instead of processes on a network.
- **Docker**: an image built from a source file into one artifact that runs
  anywhere, configured from outside by the people who deploy it.
- **Smalltalk**, to a lesser extent: a system made of objects that interact
  only by sending each other messages, kept and shipped as an image.

## Where it stands

Othismo is a work in progress, and the runtime here, `real_talk`, is a
from-scratch rewrite that started from the top: the model first, with the
semantics adjusted as they are used. Some things the premise needs aren't
there yet. A trapped object stops the whole run, calls never time out, and
there is no support yet for testing an image's objects against stand-in
environment objects.

The details of the current design are in [Namespaces](namespace.md) and
[Building images](images.md).
