# Building images

This document describes the `.oth` language images are written in, how an
image uses another, and how one is sealed into a single file for deployment,
in the way a Docker image is built from a Dockerfile. It builds on
[the namespace design](namespace.md).

## One language

Images and headers are both `.oth` files, in one language. A file can
declare:

| Declaration | What it is |
|---|---|
| `protocol`, `trait` | named shapes (see [Objects](objects.md#protocols-and-traits)) |
| `object` | the interface of a Wasm module |
| `needs <at> <shape>` | a mount point in the image's view, filled by whoever instantiates it |
| `instance` | an object, or another image, made part of this one |
| `mount`, `bind` | the views |
| `export <path>` | what an instance of the image serves |
| startup requests | requests the image makes when it starts |
| `require` | a slot: a value whoever instantiates the image gives it |

A module's header is a file that declares an object and no instances. A
deployable image is a file with instances and only needs the runtime can fill.
A test is a file that instantiates another image with stand-ins.

## Imports and exports

```text
import Site, { HttpServer as Http, FileTree } from "./site.oth"
import { Listener, Console } from env
```

- The **default import**, `Site`, is the file itself as an image: an object
  type, which `instance web : Site` makes an instance of. The name is the
  importer's choice; a file never names itself.
- The **named imports** are declarations the file exports with `export`:
  `export object`, `export trait`, `export protocol`. Anything not exported
  can't be imported, though an exported trait may use a protocol that isn't.
- Either kind can be renamed with `as`, and either can be left out: `import
  Site from ...` and `import { FileTree } from ...` are both fine.
- A file can re-export a declaration it imported, with `export { FileTree }`.
  Only declarations can be re-exported, never images.
- `env` is the runtime's own header. It has no image, so only named imports
  come from it.

Paths are relative to the importing file.

## Instances

```text
instance pages : StaticSite { mount /files files readonly }
instance files : PackedFiles { include "./public/" }
instance web   : Site { bind /net/tcp /net/tcp }
```

An instance's type is one of:

- an **object** imported from a module's header. `StaticSite`, declared in
  `sdk/objects/site/StaticSite.oth`, runs `StaticSite.wasm` beside it;
- one of the **runtime's objects** that touch nothing outside the image,
  imported from `env`: `PackedFiles`, `MemoryConsole`, `MemoryListener`;
- an **image**, imported by its default name.

No file can declare an instance of an environment object. An image gets
those through its `needs`.

Instances can be declared in any order, so an image can start from the
instance at the top of the namespace and work down.

## Views

```text
needs /net/tcp Listener                 # the image's view: filled from outside

instance http : HttpServer {
  bind  /net/tcp /net/tcp               # http's view: from the image's
  mount /www pages                      # http's view: an instance's tree
}

mount /http http                        # the image's view: an instance's tree
```

- `mount <at> <instance> [readonly]` puts that instance's whole served tree
  at `<at>`. Inside an instance's block it builds that instance's view. At the
  top level it builds the image's own view.
- `bind <at> <path> [readonly]`, inside an instance's block, gives the
  instance what `<path>` means in the image's own view. It is how an image
  passes on what it was given.
- `needs <at> <shape>`, at the top level, puts a mount point in the image's
  own view for whoever instantiates it to fill, with the shape it must have
  there.
- `export <path> [at <path>] [readonly]` serves what `<path>` means in the
  image's own view, at the same path or the one after `at`, to whoever
  instantiates the image.

## An image as an object

An instance of an image is an object made of objects, as a Plan 9 namespace
can itself be served as a tree:

```text
# site.oth: the static site, which touches nothing of its own
import { HttpServer } from "../../sdk/objects/http/HttpServer.oth"
import { StaticSite } from "../../sdk/objects/site/StaticSite.oth"
import { Listener, Console, PackedFiles } from env

needs /net/tcp  Listener
needs /dev/cons Console

# HTTP in, namespace requests out. It reaches the files only through pages.
instance http : HttpServer {
  bind  /net/tcp  /net/tcp
  bind  /dev/cons /dev/cons
  mount /www pages
}

# The pages can read the files and nothing else.
instance pages : StaticSite {
  mount /files files readonly
}

instance files : PackedFiles { include "./public/" }

mount /http http

write /http/hosts/* "/www"
write /http/hosts/files.localhost "/www/static"

# Whoever runs the site can add hosts, and nothing else.
export /http/hosts at /hosts
```

For `instance web : Site { ... }`:

- **What it needs** is the image's top-level `needs`. The instance block
  fills them with `mount` and `bind`, as it would for a Wasm object, and the
  checker holds them to the same rules.
- **What it serves** is what the image exports, and nothing else. Every
  other path is `not_found`, so nothing the image mounts only to configure it
  leaks out.
- **Its instances are private.** They are named `web.http`, `web.pages` and
  `web.files`, so they never collide with anything outside, and nothing
  outside can reach them except through what is exported.
- **Its startup requests** are made in its own view, before those of the
  image that instantiates it (see [Startup order](namespace.md#startup-order)).
- **It is the operator of its own instances.** Its startup requests carry
  the operator flag, and so do those its parent makes of it through its
  exports. No object can make one.

The checker computes reach through the instances of images as through any
others, and `--check` prints them by their full names.

## The root image

The image being run is the **root**. Its `needs` are filled by the runtime
with environment objects, chosen by the trait each one names (see
[Environment objects](namespace.md#environment-objects)):

```text
# static.oth: deploying the site
import Site from "./site.oth"
import { Listener, Console } from env

# Where to listen. Deploy with e.g. `--set listen=0.0.0.0:80`.
require listen : text/plain = "127.0.0.1:8080"

# Everything this program can do to the host: one listener, and the console.
needs /net/tcp  Listener
needs /dev/cons Console

instance web : Site {
  bind /net/tcp  /net/tcp
  bind /dev/cons /dev/cons
}

write /net/tcp listen
```

So the root's `needs` are the whole answer to "what can this program do to
the machine it runs on". An image that is imported is never the root, and
its needs are always filled by its parent, with real objects passed down or
with in-memory ones.

## Startup requests

```text
write /http/hosts/* "/www"
write /greeter/greeting application/bson { $type: "Greeting", prefix: "Hello, ", excited: true }
write /net/tcp listen
```

`<verb> <path> [payload]` is a request the image makes at startup, resolved
in the image's own view like any other request. The image can only send
requests to what its view holds, so an image with no mounts or needs sends
none. Requests can appear anywhere in the image, and are made in the order
they are written. They carry the operator flag. The image ignores answers,
except that an error is printed to stderr as a warning.

A payload is written one of these ways:

- `"text"`: a `text/plain` payload. Another text type can be named first:
  `text/html "<p>hi</p>"`.
- `<type> from "./file"`: the file's bytes, read when the image is built.
- `<type> { ... }`: a structured literal, handed to the format for that
  content type to encode. Its syntax is one for every format: strings,
  numbers, `true` and `false`, and nested `{ name: value }` and `[ ... ]`.
  The format checks the literal against the schemas of the image's modules
  when the image is built; for BSON, `$type` names the message type, and a
  type none of them declares is an error.
- the name of a slot (see below).

## Slots: values from outside

An image can require values from whoever instantiates it:

```text
require listen : text/plain = "127.0.0.1:8080"     # a default
require admin  : text/plain                        # none: it must be given
require greeting : application/bson = { $type: "Greeting", prefix: "Hello, ", excited: false }
```

- `require <name> : <type> [= <payload>]` declares a slot of a content type,
  optionally with a default. For a structured type, the default may give
  some fields and leave the rest to be given.
- A slot's name can stand wherever a payload can: as a startup request's
  payload, including an environment object's configuration. It can't change
  mounts or instance types, so a built image's views are fixed when it is
  built and `--check` stays a true upper bound.
- A slot can't share its name with an instance or another slot.

An instantiated image's slots are given in its instance block:

```text
instance web : Site {
  bind /net/tcp /net/tcp
  hosts = "example.com"
}
```

A slot the block leaves out keeps its default, and one with no default must
be given. Only the root's slots are left to whoever runs it.

### Giving the root's slots

The host gives values in layers, and a later layer wins, field by field for
a structured slot:

1. the image's defaults,
2. value files, in the order given (`--with prod.vals`),
3. `--set slot=value` or `--set slot.field=value`.

A value file uses the image's own syntax:

```text
listen = "0.0.0.0:80"
greeting = { prefix: "Hi, " }
```

Value files are relative to the working directory. `--set` takes the raw
text for a text slot or a string field, and a literal for anything else
(`--set listen=0.0.0.0:80`, `--set greeting.excited=true`).

The image refuses to start unless every slot has a value, and every field
of every structured one. It lists everything missing, e.g. `admin: required,
not provided`. A slot or field that the image doesn't declare is an error,
and so is a value the slot's format rejects. `--check` prints each slot's
final value and where it came from:

```text
$ othismo --check static.rti --set listen=0.0.0.0:80
slot listen : text/plain = "0.0.0.0:80" (--set)

from the host:
  /net/tcp    listens on 0.0.0.0:80
  /dev/cons   reads stdin, writes stdout
...
```

## Building

Running an image reads several other files: the compiled modules, each with
its header and schemas inside, the `.oth` files it imports, and whatever its
`PackedFiles` include. A **built image** (`.rti`) holds everything the image
needs except what it takes from the host through its needs, so it can be
copied to another machine and run there.

```text
othismo build static.oth -o static.rti     # seal
othismo run static.rti                     # run it, here or elsewhere
othismo static.oth                         # build in memory, then run
othismo unpack static.rti out/             # modules, files and sources
```

Running a `.oth` builds it in memory and runs the result, so there is one way
to load an image. Packed trees are a snapshot taken when the image is built;
for live edits during development, use `Files`.

### What a built image holds

- **The manifest**: the checked image, with every instance (those of
  imported images included, by their full names), every view, the startup
  requests in the order they are made, the slots and the root's needs.
- **The modules**: one compiled `.wasm` per module the image uses. Compiled
  Wasm is the only code a built image runs; `build` takes finished `.wasm`
  files and never runs the AssemblyScript compiler.
- **Each module's header and schemas**, already resolved. The schemas are
  kept as their format wrote them, for documentation and for encoding the
  slots' values; the runtime never reads them.
- **Packed file trees** for `PackedFiles` instances.
- **Sources**, for reference only: every `.oth` file used, and each module's
  schemas and source if present. Anyone who unpacks the file can read them;
  the runtime never does. `--no-sources` leaves them out.

## Packed files

`PackedFiles` serves a tree that was copied into the image when it was built.
It is the build-time counterpart of `Files`, which reads the host at run
time:

```text
# The host's files, live, in the root:
needs /files Files
write /files "/var/www"

# A snapshot, taken when the image is built, anywhere:
instance files : PackedFiles {
  include "./public/"
  include "./prod/robots.txt" at /robots.txt
}
```

`PackedFiles` touches nothing on the host when the image runs, and `--check`
reports it as built in. It answers `read`, `stat` and `list` exactly as
`Files` does (the same content types, stat records and errors). Every unsafe
verb gets `read_only`.

### `include`

Paths are relative to the image file.

- A path naming a file includes exactly that file, hidden or not.
- A path ending in `/` or `/*` includes everything under that directory,
  recursively, hidden files included. `*` means nothing anywhere else. A path
  naming a directory without the trailing `/` is an error, so a recursive
  include always says so.
- Symlinks are followed, wherever they point. A dangling link and a directory
  cycle are build errors.
- A path that doesn't exist is a build error. A directory that turns out
  empty is fine; directories are kept, empty ones included.

A directory's contents land at the root of the served tree, and a single file
lands at `/<its name>`. `at /path` puts either somewhere else.

**Later includes win.** Includes apply in order, and where two put something at
the same path, the later one replaces it:

- Directories merge. Including `./theme/` after `./public/` replaces only the
  paths `theme` has.
- A later file where a directory was replaces the whole directory, and a later
  directory where a file was replaces the file.

`build -v` lists every path an include replaced.

## Testing

Tests are objects, not part of the language. A test suite is any object whose
header declares the runtime's `TestSuite` trait:

```text
export trait TestSuite {
  /tests          list         # its tests, by name
  /tests/{name}   write        # runs one: ok if it passed, an error saying why if not
}
```

A test image is a root with no needs. It instantiates the image under test,
gives it in-memory objects where a deployment would give it real ones, and
puts a suite beside it:

```text
# examples/static/static.test.oth
import Site from "./site.oth"
import { SiteTests } from "./scripts/SiteTests.oth"
import { MemoryListener, MemoryConsole } from env

instance net  : MemoryListener
instance cons : MemoryConsole

instance web : Site {
  mount /net/tcp  net
  mount /dev/cons cons
}

instance tests : SiteTests {
  mount /net net              # the site's listener, to dial
}
```

`SiteTests` (in `examples/static/scripts/`) dials the site with one HTTP
request per test, and checks how the response begins.

```text
$ othismo test examples/static/static.test.oth
tests serves_the_index ... ok
tests missing_pages_are_not_found ... ok
...
6 passed, 0 failed
```

`othismo test` lists every suite's tests in one run of the image, then runs
each test in a fresh one: startup requests and every `start()`, then
`write /tests/<name>`, then everything that follows from it until nothing
is left to deliver. A test image has no needs, so nothing outside is
started, and a test can only reach what the image has in memory. A test that
never answers fails. What the image prints on its consoles is shown for each
test that fails, and the command exits with a failure if any did.

## The file format

```text
"RTIMAGE" 0x07        magic and format version
u64 (LE)              length of the manifest
manifest              one BSON document
blobs                 file and module contents, back to back
```

The manifest being BSON is a detail of the file, unrelated to the BSON
payload format. The version changes whenever the manifest's layout does. An
image built in another format is refused with a request to rebuild it from its
`.oth`; built images are for deploying, not for keeping across versions.

Every module, packed file and source is a blob named by the SHA-256 of its
contents, and stored once however often it appears. The manifest refers to
blobs by hash and records each one's offset and length. Building the same
inputs twice produces the same bytes: the manifest is ordered, and nothing
depends on timestamps or the order a directory is read in.

## Not yet

- Writable layers over a packed tree (needs union mounts).
- Mapping the file into memory rather than reading it whole.
- ETags from stat records' versions in the HTTP server.
