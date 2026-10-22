# Building images

This document describes how an image is sealed into a single file for
deployment, in the way a Docker image is built from a Dockerfile. It builds on
[the namespace design](namespace.md).

A `.img` file is a source. Running it reads several other files: the compiled
modules in `scripts/`, the TypeScript it imports for message types, and
whatever an `Environment.Files` root points at. A **built image** (`.rti`)
holds everything the image needs except what it explicitly takes from the
host, so it can be copied to another machine and run there.

## What a built image holds

- **The manifest**: the checked image, with its instances, views, startup
  requests and slots (see below).
- **The modules**: one compiled `.wasm` per Wasm instance type. Compiled Wasm
  is the only code a built image runs; `build` takes finished `.wasm` files
  and never runs the AssemblyScript compiler.
- **The message types**, already parsed. The runtime never reads TypeScript.
- **Packed file trees** for `Packed.Files` instances.
- **Sources**, for reference only: the `.img` itself, the `.ts` files it
  imports and each module's `scripts/<Type>.ts` if present. Anyone who
  unpacks the file can read them; the runtime never does. `--no-sources`
  leaves them out.

A built image's only contact with the host is through the environment objects
it declares (Console, Tcp, Files). Running `--check` on it gives the complete
list of what it can touch on the machine.

```text
othismo build static.img -o static.rti     # seal
othismo run static.rti                     # run it, here or elsewhere
othismo static.img                         # build in memory, then run
othismo unpack static.rti out/             # modules, files and sources
```

Running a `.img` builds it in memory and runs the result, so there is one way
to load an image. Packed trees are a snapshot taken when the image is built;
for live edits during development, use `Environment.Files`.

## Packed files

`Packed.Files` serves a tree that was copied into the image when it was built.
It is the build-time counterpart of `Environment.Files`, which reads the host
at run time:

```text
# The host's files, live:
instance files : Environment.Files { control / FilesRoot { path: "/var/www" } }

# A snapshot, taken when the image is built:
instance files : Packed.Files {
  include "./public/"
  include "./prod/robots.txt" at /robots.txt
}
```

`Packed.Files` touches nothing on the host when the image runs, and `--check`
reports it as built in.

It answers `read`, `stat` and `list` exactly as `Environment.Files` does
(`Data`, `Info`, a sorted stream of `Entry`, the same MIME types and statuses).
Every unsafe verb gets `read_only`.

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

## Slots: values from the host

An image can require a message from whoever runs it. The deployer supplies the
values; the image supplies the types and, optionally, defaults:

```text
require listen : Listen {
  address: "127.0.0.1:8080"     # a default
}
require admin : Text            # no defaults: the host must give it

instance net  : Environment.Tcp { control / listen }
instance http : HttpServer {
  control /admin admin
}
```

- `require <name> : <Type>` declares a slot. Fields written in the block are
  defaults, checked like any message literal. Fields left out must come from
  the host. No block, or an empty one, means every field must.
- A slot's name can stand wherever a message can: as a startup request's
  body, including an environment object's `control /`. It can't change mounts or instance types, so a built image's
  views are fixed when it is built and `--check` stays a true upper bound.
- A slot can't share its name with a message type or another slot.

The host gives values in layers, and a later layer wins field by field:

1. the image's defaults,
2. value files, in the order given (`--with prod.vals`),
3. `--set slot.field=value`.

A value file uses the image's own block syntax, without a type, since the
image already knows it:

```text
listen {
  address: "0.0.0.0:80"
}
```

Value files are relative to the working directory. `--set` takes the raw
text for a `string` or `ArrayBuffer` field and a literal for anything else
(`--set listen.address=0.0.0.0:80`, `--set db.port=5432`).

The image refuses to start unless every field of every slot has a value. It
lists every missing field, e.g. `admin.text: required, not provided`. A slot
or field that the image doesn't declare is an error, and so is a value of the
wrong type. `--check` prints each slot's final value and where each field came
from, then what each environment object touches on the host:

```text
$ othismo --check static.rti --set listen.address=0.0.0.0:80
slot listen : Listen { address: "0.0.0.0:80" (--set) }

net    listens on 0.0.0.0:80
cons   reads stdin, writes stdout
files  serves 10 built-in files, 95.0 KB
...
```

A relative `FilesRoot` path resolves against the directory holding the image,
`.img` or `.rti`.

## The file format

```text
"RTIMAGE" 0x03        magic and format version
u64 (LE)              length of the manifest
manifest              one BSON document
blobs                 file and module contents, back to back
```

The version changes whenever the manifest's layout does. An image built in
another format is refused with a request to rebuild it from its `.img`;
built images are for deploying, not for keeping across versions.

Every module, packed file and source is a blob named by the SHA-256 of its
contents, and stored once however often it appears. The manifest refers to
blobs by hash and records each one's offset and length. Building the same
inputs twice produces the same bytes: the manifest is ordered, and nothing
depends on timestamps or the order a directory is read in.

## Not yet

- Writable layers over a packed tree (needs union mounts).
- Mapping the file into memory rather than reading it whole.
- ETags from the content hashes in the HTTP server.
