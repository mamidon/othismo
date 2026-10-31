# Objects

An object is a WebAssembly module running in a sandbox. The only thing it can
reach is its namespace: the paths it has been given. Whatever it does, it does
by answering requests and making them.

## What an object is

A module is a compiled `.wasm` file, with a header describing it. An object is
one running instance of a module, declared in an image. In the
[tour](tour.md#siteoth), `site.oth` makes three of them: `http`, `pages` and
`files`.

Two instances of the same module share code and nothing else. Each has its
own memory and its own namespace.

## Server and client at once

Every object is both a server and a client. It serves a tree of paths, and
answers the requests made of them. It also has a view, the paths it has been
given, and it makes requests of those.

Its header says both: what it `serves`, and what it `needs` in its view. The
tour walks through [HttpServer's header](tour.md#httpservers-header). Nothing
else about an object is visible from outside: not its memory, not its
functions, not the language it was written in.

## No ambient access

An object has no files, no network, no clock and no randomness and so on.

Objects can only interact with other objects through the namespace. 
If it has a socket, it's because something put one in its view, and
you can see where in the image. In the tour, that's the `needs` lines in 
[`static.oth`](tour.md#staticoth): the socket and the console come 
from the environment, and are passed down from there as needed.

## Single-threaded, sharing nothing

Objects share no memory. What passes between them is copied bytes.

Othismo currently runs as a single threaded message queue.

## Protocols and traits

To enable code re-use and shared behaviors, Othismo supports `protocols` and `traits`.

A **protocol** describes the requests a single path takes:

```text
## Something read whole.
protocol File { read -> */*, stat }

## A stream of bytes each way.
protocol Connection { watch -> application/octet-stream*, write application/octet-stream }
```

A **trait** describes the shape of a whole tree, in the same grammar as `serves`,
and can use protocols for its paths:

```text
## A listener, and the connections it accepts.
trait Listener {
  /                operator write text/plain
  /address         read -> text/plain
  /accept          watch -> text/plain*
  /{conn}/data     Connection
  /{conn}/remote   read -> text/plain
  /{conn}          remove
}
```

A trait can be used in two places. `needs /net/tcp Listener` says the object
needs a whole Listener at `/net/tcp`. `object StaticSite : FileTree` says
StaticSite serves a FileTree, alongside whatever else it serves.

Traits are names for shapes, not types to inherit from. HttpServer's
console need is written out in its header rather than named as `Console`, and
the real console still fits it.

This is also how stand-ins work. The runtime's console is a `Console`, and so
is `MemoryConsole`, which touches nothing on the host. An image that needs a
Console can be given either. That's how an image is tested.

The runtime's own traits, `Console`, `Listener`, `Files`, `FileTree` and the
rest, are declared in its header, `env`.
