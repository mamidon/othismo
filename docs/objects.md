# Objects

An object is a WebAssembly module, running in a sandbox whose only way out is
its namespace, and it comes with a header that says exactly what it serves
and what it needs. The header is what you review; the runtime holds the code
to it.

## What an object is

A **module** is a compiled `.wasm` file. An **object** is one running
instance of a module, declared in an image:

```text
import { StaticSite } from "../../sdk/objects/site/StaticSite.oth"

instance pages : StaticSite {
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

Each module has a header next to its source, `StaticSite.oth`,
written in the same language as images. It declares the module's object: what
it serves, what it needs, and its documentation. The module itself is the
`.wasm` with the same name beside it, `StaticSite.wasm`.

```text
import { FileTree } from env

## Serves a static site from a files tree it is given read-only.
##
## Requests for paths ending in `/` get `index.html` added. It forwards each
## safe request to `/files` and never sees the bytes it serves.
export object StaticSite : FileTree {
  ## The site's files. Nothing is ever written here.
  needs /files FileTree

  serves {
    ## How many requests it has forwarded, as decimal text.
    /hits   read -> text/plain
  }
}
```

`FileTree` is a trait, the shape of a tree of files, from the runtime's own
header (see [Protocols and traits](#protocols-and-traits)). An object must be
`export`ed for an image to import it.

A header describes paths, verbs and content types, never what is inside a
payload. What a module's payloads hold is described by its schemas, in its
payload format's own language (see [Payloads](#payloads)).

### Serves and needs

`serves` lists the paths an object answers for. `needs` lists, for each mount
point in its view, what it expects to find there. They use one grammar: a
path pattern followed by the requests it takes.

```text
needs /net/tcp {
  /address        read -> text/plain
  /accept         watch -> text/plain*
  /{conn}/data    watch -> application/octet-stream*, write application/octet-stream
  /{conn}         remove
}
```

Each request is:

- optionally `operator`, for a request only whoever runs the image may make;
- a verb;
- the content type of its payload, if it takes one;
- after `->`, the content type its answer carries, with `*` if the answer is
  a stream.

A request with no arrow, such as `remove`, is answered with a status alone.
`stat` and `list` are written bare: they always answer stat records. `Stat`
names a stat record's content type wherever another answer carries one, as
`create` conventionally does.

Content types can be wildcards: `*/*` is any payload and `text/*` any text.
A server that takes `write */*` meets a need to `write text/plain`, and a
need for `read -> */*` is met by a server that answers anything. The reverse
doesn't hold: a need to send `*/*` is met only by a server that takes
`*/*`, and a need for `-> text/plain` isn't met by a server that promises
only `-> */*`.

Paths are patterns:

- A literal, such as `/hits` or `/hosts/`, matches exactly that path. A
  trailing `/` counts, as it does everywhere else.
- `{name}` matches exactly one segment. The name is for the documentation and
  for generated code, where it becomes an argument.
- `**` matches the rest of the path, zero or more segments, and only comes
  last. `/www/**` matches `/www` itself as well as everything below it, and
  `/**` matches every path, `/` included.

Where several patterns match, the most specific wins, whatever their order: a
literal over `{name}`, `{name}` over `**`, and longer over shorter. It is the
same rule that picks a mount point.

A `needs` block's own path is a pattern too, for objects that are told at run
time which mount points to use. `needs /sites/{name}` covers every mount point
directly under `/sites/`, and `needs /sites/**` covers `/sites` and any mount
point below it. HttpServer is configured with a mount point for each host,
which can be anywhere, so it needs every mount it isn't given a more specific
need for:

```text
needs /net/tcp Listener
needs /dev/cons { / write text/plain }

## The sites, at whichever mount points the hosts are configured with.
needs /** {
  /**     read -> */*, stat
}

serves {
  ## The hosts it serves.
  /hosts           list
  ## Write a path in its view to serve the host from there, or remove it to
  ## stop. `*` stands for any host not listed.
  /hosts/{name}    operator write text/plain, operator remove, read -> text/plain
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
protocol File { read -> */*, stat }

## Something listed.
protocol Dir { list, stat }
```

A **trait** names the shape of a whole tree, in the grammar of `serves`:

```text
## A tree of files, read-only.
export trait FileTree {
  /**   File, Dir
}

## A listener, and the connections it accepts.
export trait Listener {
  /               operator write text/plain
  /address        read -> text/plain
  /accept         watch -> text/plain*
  /{conn}/data    Connection
  /{conn}/remote  read -> text/plain
  /{conn}         remove
}
```

They stand in for what they name:

- A protocol's name goes wherever requests do, alone or among them:
  `/{conn}/data Connection`, or `/x File, write text/plain`. Two protocols
  that list the same request must agree on its answer.
- A trait's name stands alone at a path, and puts its whole tree there:
  `/static FileTree` in a trait or a `serves` block covers `/static/**`.
  It can't go under `**`, which only comes last.
- `needs /net/tcp Listener` needs the trait's whole tree at that mount point.
- `object StaticSite : FileTree` declares that StaticSite serves FileTree:
  the trait's entries are among what it serves, alongside its own.

Where two traits, or a trait and an object's own `serves`, give the same
pattern, they must take exactly the same requests. Anything else is an
error, rather than one silently winning.

Protocols and traits are exported and imported like objects, `import {
FileTree } from env`, and can be declared in any order. One name means one
thing in a file. A trait that is exported may use a protocol that isn't:
the protocol's requests are written out into the trait, and importers never
see its name.

They change nothing about how mounts are checked. Matching stays structural:
a mount meets `needs /files FileTree` if what is mounted serves the trait's
entries, whether or not its header says `: FileTree`.

### Shapes, not behaviour

A header describes shapes: which paths exist, which verbs they take, and which
content types go in and come out. It can't describe behaviour, such as
whether a `read` really changes nothing, or when an object answers
`not_found`. Nor does it say which message is inside an `application/bson`
payload; that is the payload format's business, and the entry's
documentation should say.

Headers don't list the errors a request can return. Any request may fail
with any error. Declaring them would be checked exceptions by another name,
and would promise something nothing can enforce.

### Checking the wiring

The checker reads every object's header and checks the images against them:

- Each mount must provide what the object needs there. Whatever is mounted at
  `/files` must serve everything `needs /files` lists, and a mount that is
  read-only can't provide a need to change anything.
- A need for one mount point in particular, such as `needs /files`, must have
  something mounted there. One under a pattern, such as `needs /**`, may
  match no mount at all.
- Each startup request must match an entry in what it reaches.

Matching is structural. A mount satisfies a need if the mounted object serves
each listed request, with the same verb, a content type that fits, and an
answer whose content type fits, under a pattern at least as wide. What the
object is called doesn't matter. Anything that serves the right paths can
stand in, which is also how an object is tested: against a fake that serves
what it needs.

These are errors in the image, reported when it is checked or built.

### Enforcement

TypeScript's types are gone by the time its code runs. Othismo's don't have to
be: every request crosses a boundary the runtime controls, so it can hold
objects to their headers while they run, without trusting their code.

- A request that reaches an object must match its `serves`.
- A request an object sends must match its `needs`: the block for the mount
  point it goes through, and an entry there. A mount point the object binds
  for itself needs a block too.
- An operator request must come from an image, and a request an entry marks
  `operator` must be one.
- Only a stream may have more than one answer.

A request that doesn't fit is refused before it arrives: `not_found` if no
pattern matches, `not_supported` if the pattern doesn't take that verb, and
`forbidden` if it breaks the operator rule. An answer that breaks the rules is
the answering object's fault, and stops the run.

The runtime never looks inside a payload, and doesn't hold one to the content
types a header gives. A server that is sent a content type it doesn't take
answers `unsupported_type`, and the code generated from its header does that
for it.

The `needs` side is the stronger promise. Mounts say what an object has been
given; its `needs` say the most it can ask of each. An object given the
console to write a log can't watch it for input unless its header says so.

Enforcement is on by default; `--no-checks` turns it off. The operator rule
stays on, as the read-only rule does: they decide what an object can reach,
not just what its header says.

### Documentation

Headers carry their documentation, so it travels with the module and can be
read or generated from it.

- A `##` comment documents the declaration after it: an object, a trait, a
  `needs` block or a `serves` entry. It is Markdown, and its first sentence
  is the summary. A plain `#` comment is just a comment.
- An object's **guide**, its man page, is the file named by
  `guide "./StaticSite.md"` in the object block. Without one, the object's
  own `##` comment is the guide. Small objects need only the comment; large
  ones keep a paragraph there and the rest in Markdown.

`othismo doc` prints a module's guide followed by a reference generated from
its declarations and its schemas, and the project's reference pages are built
the same way.

### Built-in objects

The runtime's traits (`Console`, `Listener`, `Files`, `FileTree`, `Terminal`,
`Dialer`, `TestSuite`) and its objects (`MemoryConsole`, `MemoryListener`,
`PackedFiles`) are declared in its own header, `env`, like any other. The
checker treats them the same way, and their reference documentation comes
from that header.

## Payloads

What a module sends and receives in its payloads is described by its
**schemas**, written in the language of the payload's format. For BSON that
is a file of message types next to the header, `scripts/Greeter.messages`:

```text
## Configures a Greeter; sent once at startup by the image.
message Greeting {
  ## Put before each name.
  prefix: string
  ## End with `!` rather than `.`.
  excited: bool
}
```

The header only says that `/greeting` takes `application/bson`, and its
documentation says the payload is a `Greeting`. [The protocol](protocol.md#layer-2-payload-formats)
covers formats, BSON's field types, and when two modules' schemas for one
type agree.

## The sandbox

A module may import the runtime's own functions and nothing else, apart from
`env.abort`, which AssemblyScript always emits. A module that imports
anything more, such as WASI, a clock, or the source of randomness behind
`Math.random`, is refused before it runs.

So an object has no files, no sockets, no clock, no randomness and no
threads. It can't find out anything except by asking through its view, and it
can't affect anything except by answering or asking. What it does depends
only on the requests and answers it receives.

Each object has its own Wasmtime store and its own linear memory. Objects
share nothing; what passes between them is copied bytes.

CPU time and memory aren't limited yet. An object can loop forever or grow
until the host runs out.

## The contract with the runtime

The interface between a module and the runtime is small, and the same for
every language: [the host ABI](protocol.md#the-host-abi).

A module can **call out** to make a request of a path in its view, to answer
a request it is serving, to forward a request elsewhere, to give up on one of
its own, and to rearrange its view. The runtime **calls in** to deliver a
request, to deliver an answer, to start the object, and to tell it a caller
gave up.

Nothing is serialized by the runtime. The verb, path, flags, status and the
numbers that tie answers to requests travel as plain arguments, and a
payload's content type and bytes are copied as they are. The runtime reads
only the bytes a module points it at, and writes only into a buffer the
module hands it.

### One thing at a time

Othismo is currently single-threaded, with one queue delivering requests and
answers for the whole image. Each delivery runs to completion, and the next
starts only when it returns. An object never has to think about concurrency,
and nothing it does can be interrupted by another object.

A call returns at once with a ticket. Its answer arrives later, as its own
delivery, and the ticket says which call it answers. A request can be
answered several times, as a **stream**; each answer says whether it is the
last. A caller can cancel, and later answers are dropped.

An object can also **forward** a request it is serving to a path in its own
view. The answer then goes straight to the original caller, and the object
never sees it. That is how StaticSite serves files without touching their
bytes.

## Building a module

A module is built from its header, its schemas and its source:

1. Bindings are generated for the module's language, one file from each:
   - from the header, `scripts/Greeter.oth.ts`: a server base class for what
     it serves, and a client for each of its needs;
   - from the schemas, by the format's generator, `scripts/Greeter.messages.ts`:
     a class for each message type, with its encoder and decoder.
2. The source is compiled to `.wasm`.
3. The header and the schemas beside it are appended to the module, as
   custom sections: `othismo embed scripts/Greeter.oth scripts/Greeter.wasm`.

`task` does all three for the examples. The generated bindings are ordinary
files, committed next to the source, so editors and compilers see them like
any other source without a build first. Generating them means a path, a
verb or a type is written once and the code can't drift from it; a build
regenerates them, so a stale binding shows up as a diff, and the tests fail
on one.

Because the header and the schemas are inside the module, a compiled object
describes itself. It can be published as one file, and another module's
build can generate clients and payload classes for it from the `.wasm`
alone.

## AssemblyScript

Modules are written in AssemblyScript today. It reads like TypeScript,
compiles to small modules and needs no runtime of its own. Nothing in the
contract above depends on it, so any language that compiles to WebAssembly
and has bindings generators can be used. A Rust module would generate its
bindings from the same header and schemas.

The library has three parts, covered in [the SDK](protocol.md#the-sdk):

- `othismo`: `Server`, `Request`, `Response` and `Payload`, and `ns`, which
  makes requests and rearranges the view.
- `othismo/bson`: BSON messages and their encoding.
- `othismo/env`: clients and server bases for the runtime's traits, generated
  from `env`'s header.

Some of its shape comes from AssemblyScript's limits. There are no closures
that capture variables, so an answer is delivered to a `Then` object that
holds whatever state it needs in its fields. There are no union types, so a
decoded message is told apart with `instanceof`. A module that receives a
BSON message it has no class for gets an `Unknown` one, which it can't read
but can pass on unchanged, as it can any payload.

## Failure

A trap or abort in any object stops the whole run, with an error naming the
object. Whether a failing object should be contained, so that only its
callers notice, is open.

Calls never time out. A request whose server never answers waits for as long
as the image runs.

## Where it stands

Headers, schemas and bindings work as described here. Every example object
has a header and is written against the bindings generated from it, the
checker holds images to their objects' headers, and the runtime holds the
objects to them while they run.

AssemblyScript is the only language with bindings generators so far, and
BSON the only payload format. The checker doesn't model an object's own
exposures: a mount is checked against what its target serves, not what the
target might expose there at run time, so only the runtime catches a
mismatch behind one.
