# Objects

An object is a WebAssembly module, running in a sandbox whose only way out is its 
namespace, and it comes with a header that says exactly what it serves and what it needs.
The header is what you review; the runtime holds the code to it.

## What an object is

A **module** is a compiled `.wasm` file, named for the type it implements:
`StaticSite` is `scripts/StaticSite.wasm`. An **object** is one running
instance of a module, declared in an image:

```text
instance site : StaticSite {
  mount /files files readonly
}
```

Two instances of the same type share code and nothing else. Each has its own
memory, its own view and its own served tree.

Every object is two things at once. It is a **server**, answering requests made
of its served tree, and a **client**, making requests of the paths in its view.
Nothing else about it is visible from outside: not its memory, not its
functions, not the language it was written in.

## The interface

Each module has a header, an `.iface` file written in the same language as
images, next to its source: `scripts/StaticSite.iface`. It is the module's
interface: the message types it uses, what it serves, what it needs, and its
documentation.

```text
import { Data, FileTree } from env

## Serves a static site from a files tree it is given read-only.
##
## Requests for paths ending in `/` get `index.html` added. It forwards each
## safe request to `/files` and never sees the bytes it serves.
object StaticSite : FileTree {
  ## The site's files. Nothing is ever written here.
  needs /files FileTree

  serves {
    ## How many requests it has forwarded, as decimal text.
    /hits   read -> Data
  }
}
```

`FileTree` is a trait, the shape of a tree of files, from the runtime's own
header (see [Protocols and traits](#protocols-and-traits)).

### Message types

A message type is a name and a list of fields:

```text
## Configures a Greeter.
message Greeting {
  ## Put before each name.
  prefix: string
  ## End with `!` rather than `.`.
  excited: bool
}
```

Fields are flat: strings, booleans, sized integers and floats, and bytes.
There are no nested messages, arrays or optional fields yet.

On the wire a message is identified only by its name, so a type *is* its
name together with its exact fields. Two headers may both declare `Greeting`
if they agree on its fields; if they disagree, an image can't use both.

A header can import types from other headers, and from `env`, which holds the
runtime's own types.

`Opaque` is reserved and means any message. Console prints whatever it is
sent, so it serves `/ write Opaque`; a logging proxy that passes on whatever
comes back answers `-> Opaque*`. In matching, `Opaque` goes both ways: a server
that takes `write Opaque` satisfies a need to `write Line`, and a need for
`read -> Opaque` is satisfied by a server that answers any type. The reverse
doesn't hold: a need to send `Opaque` is met only by a server that takes
`Opaque`, and a need for `-> Line` isn't met by a server that promises only
`-> Opaque`.

### Serves and needs

`serves` lists the paths an object answers for. `needs` lists, for each mount
point in its view, what it expects to find there. They use one grammar: a
path pattern followed by the requests it takes.

```text
needs /net/tcp {
  /address   read -> Text
  /accept    watch -> Accepted*
  /{conn}    watch -> Data*, write Data, remove
}
```

Each request is a verb, the body type if the verb takes one, and after `->`
the type its answer carries. `Accepted*` is a stream. A request with no arrow,
such as `remove`, is answered with a status alone.

Paths are patterns:

- A literal, such as `/hits` or `/hosts/`, matches exactly that path. A
  trailing `/` counts, as it does everywhere else.
- `{name}` matches exactly one segment. The name is for the documentation; it
  captures nothing, since the object gets the whole path anyway.
- `**` matches the rest of the path, zero or more segments, and only comes
  last. `/www/**` matches `/www` itself as well as everything below it, and
  `/**` matches every path, `/` included.

Where several patterns match, the most specific wins, whatever their order: a
literal over `{name}`, `{name}` over `**`, and longer over shorter. It is the
same rule that picks a mount point.

A `needs` block's own path is a pattern too, for objects that are told at run
time which mount points to use. `needs /sites/{name}` covers every mount point
directly under `/sites/`, and `needs /sites/**` `/sites` and any mount point
below it.
HttpServer is configured with a mount point for each host, which can be
anywhere, so it needs every mount it isn't given a more specific need for:

```text
needs /net/tcp Listener
needs /dev/cons { / write Opaque }

## The sites, at whichever mount points the hosts are configured with.
needs /** {
  /**     read -> Data, stat -> Info, list -> Entry*
}
```

Every mount the pattern matches must provide what the block lists. Where
several blocks match a mount point, the most specific applies, so `/net/tcp`
is held to its own block and not to `/**`.

### Protocols and traits

The same requests and the same trees come up again and again: everything that
serves files answers `read`, `stat` and `list` the same way. A header can name
them once.

A **protocol** names the requests one path takes:

```text
## Something read whole.
protocol File { read -> Data, stat -> Info }

## Something listed.
protocol Dir { list -> Entry*, stat -> Info }
```

A **trait** names the shape of a whole tree, in the grammar of `serves`:

```text
## A tree of files, read-only.
trait FileTree {
  /**   File, Dir
}

## A listener, and the connections it accepts.
trait Listener {
  /           control Listen
  /address    read -> Text
  /accept     watch -> Accepted*
  /{conn}     Connection
}
```

They stand in for what they name:

- A protocol's name goes wherever requests do, alone or among them:
  `/{conn} Connection`, or `/x File, write Data`. Two protocols that list
  the same request must agree on its answer.
- A trait's name stands alone at a path, and puts its whole tree there:
  `/static FileTree` in a trait or a `serves` block covers `/static/**`.
  It can't go under `**`, which only comes last.
- `needs /net/tcp Listener` needs the trait's whole tree at that mount point.
- `object StaticSite : FileTree` declares that StaticSite serves FileTree:
  the trait's entries are among what it serves, alongside its own.

Where two traits, or a trait and an object's own `serves`, give the same
pattern, they must take exactly the same requests. Anything else is an
error, rather than one silently winning.

Protocols and traits are imported like message types, `import { FileTree }
from env`, and can be declared in any order. One name means one thing in a
header: a message type, a protocol or a trait.

They change nothing about how mounts are checked. Matching stays structural:
a mount meets `needs /files FileTree` if what is mounted serves the trait's
entries, whether or not its header says `: FileTree`.

### Shapes, not behaviour

A header describes shapes: which paths exist, which verbs they take, and which
types go in and come out. It can't describe behaviour, such as whether a
`read` really changes nothing, or when an object answers `not_found`.

Headers don't list the failures a request can return. Any request may
fail with any status. Declaring them would be checked exceptions by another
name, and would promise something nothing can enforce.

### Checking the wiring

The checker reads every object's header and checks the image against them:

- Each mount must provide what the object needs there. Whatever is mounted at
  `/files` must serve everything `needs /files` lists, and a mount that is
  read-only can't provide a need to change anything.
- A need for one mount point in particular, such as `needs /files`, must have
  something mounted there. One under a pattern, such as `needs /**`, may
  match no mount at all.
- Each startup request must match an entry in the object's `serves`.

Matching is structural. A mount satisfies a need if the mounted object serves
each listed request, with the same verb and body type and an answer of the
same type, under a pattern at least as wide. What the object is called
doesn't matter. Anything that serves the right paths can stand in, which is
also how an object is tested: against a fake that serves what it needs.

These are errors in the image, reported when it is checked or built.

### Enforcement

TypeScript's types are gone by the time its code runs. Othismo's don't have to
be: every message crosses a boundary the runtime controls, so it can hold
objects to their headers while they run, without trusting their code.

- A request that reaches an object must match its `serves`.
- A request an object sends must match its `needs`: the block for the mount
  point it goes through, and an entry there. A mount point the object binds
  for itself needs a block too.
- An answer must carry the type its entry promised, and only a stream may
  have more than one.
- Every body must be a type some header in the image declares, with exactly
  its fields.

A request that doesn't fit is refused before it arrives: `not_found` if no
pattern matches, `not_supported` if the pattern doesn't take that verb, and
`invalid` if the body is the wrong type. An answer of the wrong type is the
answering object's fault.

The `needs` side is the stronger promise. Mounts say what an object has been
given; its `needs` say the most it can ask of each. An object given the
console to write a log can't watch it for input unless its header says so.

Enforcement is on by default; `--no-checks` turns it off. A malformed body
stops the run, as an answer of the wrong type does.

### Documentation

Headers carry their documentation, so it travels with the module and can be
read or generated from it.

- A `##` comment documents the declaration after it: an object, a message, a
  field, a `needs` block or a `serves` entry. It is Markdown, and its first
  sentence is the summary. A plain `#` comment is just a comment.
- An object's **guide**, its man page, is the file named by
  `guide "./StaticSite.md"` in the object block. Without one, the object's
  own `##` comment is the guide. Small objects need only the comment; large
  ones keep a paragraph there and the rest in Markdown.

`othismo doc` prints a module's guide followed by a reference generated from
its declarations, and the project's reference pages are built the same way.

### Built-in objects

Console, Tcp, Files and `Packed.Files` have headers like any other object,
shipped with the runtime. The checker treats them the same way, and their
reference documentation comes from those headers.

## The sandbox

A module may import the runtime's own functions and nothing else, apart from
`env.abort`, which AssemblyScript always emits. A module that imports
anything more, such as WASI, a clock, or the source of randomness behind
`Math.random`, is refused before it runs.

So an object has no files, no sockets, no clock, no randomness and no
threads. It can't find out anything except by asking through its view, and it
can't affect anything except by answering or asking. What it does depends
only on the messages it receives.

Each object has its own Wasmtime store and its own linear memory. Objects
share nothing; what passes between them is copied message bytes.

CPU time and memory aren't limited yet. An object can loop forever or grow
until the host runs out.

## The contract with the runtime

The interface between a module and the runtime is small, and the same for
every language.

A module can **call out** to make a request of a path in its view, to reply to
a request it is serving, to forward a request elsewhere, to give up on one of
its own, and to rearrange its view. The runtime **calls in** to deliver a
request, to deliver an answer, to start the object, and to tell it a caller
gave up.

Only bodies are serialized. The verb, path, status and the numbers that tie
answers to requests travel as plain arguments, so the runtime can route
requests and enforce rules without ever decoding a body. Bodies are BSON
documents carrying their type name. The runtime reads only the bytes a module
points it at, and writes only into a buffer the module hands it.

### One thing at a time

Othismo is currently single threaded, with one queue delivering messages for 
the whole image, each delivery runs to completion, and the next starts only
when it returns. An object never has to think about concurrency, and nothing
it does can be interrupted by another object.

A call returns at once with a ticket. Its answer arrives later, as its own
delivery, and the ticket says which call it answers. A request can be
answered several times, as a **stream**; each answer says whether it is the
last. A caller can cancel, and later answers are dropped.

An object can also **forward** a request it is serving to a path in its own
view. The answer then goes straight to the original caller, and the object
never sees it. That is how StaticSite serves files without touching their
bytes.

### Startup

1. Every object is created and its view built from the image's mounts. Each
   module's top-level code runs, and may already make requests, which wait in
   the queue.
2. The image's startup requests are delivered: `control` requests and any
   others, in the order the image writes them.
3. Each object's `start` runs, after everything it mounts.
4. The environment starts: stdin is read and sockets accept connections.

The run ends when the queue is empty and nothing outside can add to it.

## Building a module

A module is built from its header and its source:

1. Bindings are generated from the `.iface` for the module's language:
   classes for its message types, with their encoders and decoders.
   `othismo bindings scripts/Greeter.iface` writes `scripts/Greeter.iface.ts`,
   which `Greeter.ts` imports as `"./Greeter.iface"`.
2. The source is compiled to `.wasm`.
3. The header is appended to the module as a custom section:
   `othismo embed scripts/Greeter.iface scripts/Greeter.wasm`.

`task` does all three for the examples. The generated bindings are ordinary
files, committed next to the source, so editors and compilers see them like
any other source without a build first. Generating them means a type is
written once and its encoding can't drift from its declaration; a build
regenerates them, so a stale binding shows up as a diff, and the tests fail
on one.

Because the header is inside the module, a compiled object describes itself.
It can be published as one file, and an image can import a type straight
from it:

```text
import { Greeting } from Greeter
```

An image imports only the types it writes in its own messages, such as a
startup request's body. It no longer has to import anything on its objects'
behalf.

## AssemblyScript

Modules are written in AssemblyScript today. It reads like TypeScript,
compiles to small modules and needs no runtime of its own. Nothing in the
contract above depends on it, so any language that compiles to WebAssembly
and has a bindings generator can be used. A Rust module would generate its
bindings from the same header.

The library has three parts:

- `othismo`: messages and their encoding.
- `othismo/ns`: `Server`, with `serve` and `start` to override, `Request`
  and `Response`, and `ns`, which makes requests and rearranges the view.
- `othismo/env`: the runtime's message types, generated from `env`'s
  header.

Some of its shape comes from AssemblyScript's limits. There are no closures
that capture variables, so an answer is delivered to a `Then` object that
holds whatever state it needs in its fields. There are no union types, so a
body is told apart with `instanceof`. A module that receives a type it has no
binding for gets an `Opaque` message, which it can't read but can pass on
unchanged.

## Failure

A trap or abort in any object stops the whole run, with an error naming the
object. Whether a failing object should be contained, so that only its
callers notice, is open.

Calls never time out. A request whose server never answers waits for as long
as the image runs.

## Where it stands

Headers work as described here. Every example object has one, the bindings
are generated from them, the checker holds images to them, and the runtime
holds the objects to them while they run.

AssemblyScript is the only language with a bindings generator so far. The
checker doesn't model exposures: a mount is checked against what its target
serves, not what the target might expose there at run time, so only the
runtime catches a mismatch behind an exposure.
