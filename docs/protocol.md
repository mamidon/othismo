# The protocol

Objects reach each other in three layers. The namespace decides where a
path leads, the protocol says what can be asked of it, and a payload format
says what the bytes in a request or an answer mean. Othismo owns the first
two and keeps them small and fixed; the third belongs to the objects.

## Three layers

| Layer | What it is | Who understands it |
|---|---|---|
| 0, the namespace | views, mounts, `bind`, `expose`, resolving a path | the runtime |
| 1, the protocol | verb, path, flags, status, streams, cancel, forward | the runtime and every object |
| 2, payload formats | the bytes in a request or an answer, and what they mean | the objects that exchange them |

The split is the one 9P and HTTP both make. 9P is a short list of messages
(`Tread`, `Twrite`, `Tstat`, ...), and what each file server says through
reads and writes, such as the commands written to `/net/tcp/clone` or a
`ctl` file, is that server's business. HTTP is a method, a path and headers,
and the body is whatever its `Content-Type` says it is.

Layers 0 and 1 change rarely and on purpose. A new kind of object, a new
message type or a new payload format changes neither: the runtime routes a
payload by its path and verb, never by what is inside it.

## Layer 0: the namespace

Every object has a **view**, the paths it can send requests to, and a
**served tree**, the paths it answers for. The image builds each view;
an object can rearrange its own afterwards, but only using what it can
already see. [Namespaces](namespace.md) covers the model, how paths
resolve, and what the checker can work out from it.

| Othismo | Plan 9 | Semantics |
|---|---|---|
| in an image: `mount <at> <instance> [readonly]` | `mount(fd, afd, old, flag, aname)`, which makes the kernel send `Tattach` | Attach an instance's whole served tree at a point in a view. Only an image can do it. |
| in an instance block: `bind <at> <path>` | `bind(name, old, flag)` | Give an instance what `<path>` means in the image's own view. |
| `bind(src, at, flags)` → status | `bind(name, old, flag)` | Make `at` in the caller's view mean whatever `src` means now. Later changes to `src` don't affect it. |
| `unmount(at)` → status | `unmount(name, old)` | Take a mount out of the caller's view. |
| `expose(at, src, flags)` → status | none; `exportfs` serves a whole namespace over 9P | Splice part of the caller's view into the tree it serves. Requests under `at` go straight to `src` and never reach the caller's `serve`. |
| in an image: `export <path> [at <path>] [readonly]` | none | What an instance of the image serves: part of the image's view, at a path of its choosing. |
| each instance gets the view the image built for it | `rfork(RFNAMEG)`, `rfork(RFCNAMEG)` | Where a view comes from. Plan 9 shares, copies or starts a process's namespace afresh. Othismo always builds it from the image, and never inherits one. |

The flags are `READONLY`, and later `BEFORE` and `AFTER` for union mounts.
A read-only mount, bind or exposure lets only safe verbs through, and the
restriction only accumulates along a chain. Plan 9 has `MCREATE` as well,
and no read-only flag.

Nothing here is a message. `bind`, `unmount` and `expose` are host calls an
object makes about its own view, and `mount`, `bind` in an instance block,
and `export` are lines in an image.

## Layer 1: the protocol

A request is a verb, a path in the sender's view, flags, and optionally a
payload. An answer is a status and, for a successful one, optionally a
payload. Every request gets at least one answer; several, if it is a stream.

```text
request   ticket verb path flags [payload]
answer    ticket ok done [payload]
          ticket error errno code description
payload   type[s] length[4] bytes[length]
```

`[s]` is a string with its length first, `[n]` is n bytes, as in the 9P
manual. A payload's `type` is a content type (see
[Content types](#content-types)); the runtime carries it and the bytes
without looking at either.

There are no handles. Every request carries its whole path, and the runtime
resolves it afresh each time. That is what 9P's `walk`, `open` and `clunk`
are for, and Othismo doesn't need them: the runtime's resolution and the
server's own reading of the path do the walk, the checks `open` would make
are made on every request, and there is nothing to clunk.

### Operations

Each 9P message starts with `size[4] type[1] tag[2]`, and those are left out
here, as are the ticket, verb, path and flags every Othismo request starts
with.

| Othismo | 9P | Semantics |
|---|---|---|
| none; the image's `mount` (Layer 0) | `Tattach fid[4] afid[4] uname[s] aname[s]` → `Rattach qid[13]` | Connect to a server's tree as a user; the fid names its root. |
| none; every request carries its whole path | `Twalk fid[4] newfid[4] nwname[2] nwname*(wname[s])` → `Rwalk nwqid[2] nwqid*(qid[13])` | Resolve names one at a time from a fid; `newfid` names the result. |
| none; the read-only rule is applied to every request | `Topen fid[4] mode[1]` → `Ropen qid[13] iounit[4]` | Prepare a fid for I/O in a mode; permissions are checked here. |
| none; nothing to release, and a stream ends with `cancel` | `Tclunk fid[4]` → `Rclunk` | Forget a fid, and any state the server attached to it. |
| `read` → `ok done payload` | `Tread fid[4] offset[8] count[4]` → `Rread count[4] data[count]` | Othismo: all of what is at the path, once, with its content type. 9P: up to `count` bytes from `offset`; reading a directory gives stat records. Safe. |
| `watch` → `ok more payload`, repeated, then `ok done` | none; repeated `Tread` of a file that blocks, such as `/dev/cons` | Payloads as they happen, until the server ends the stream or the caller cancels it. Safe. |
| `list` → `ok more stat`, repeated, then `ok done` | `Tread` of a directory fid → `Rread` of `stat[n]` records | What is directly under the path, a stat record each. Safe. |
| `stat` → `ok done stat` | `Tstat fid[4]` → `Rstat stat[n]` | What is at the path: see [Stat records](#stat-records). Safe. |
| none | `Twstat fid[4] stat[n]` → `Rwstat` | Change metadata: rename, change mode, truncate. |
| `write [payload]` → `ok done [payload]`, or a stream | `Twrite fid[4] offset[8] count[4] data[count]` → `Rwrite count[4]` | Othismo: change what is at the path or act on it. The server may answer with a payload, which belongs to this request alone, so a write and its result can't be split by anyone else's write. 9P: bytes at an offset; answers how many were written. |
| `create [payload]` → `ok done [payload]`, or a stream | `Tcreate fid[4] name[s] perm[4] mode[1]` → `Rcreate qid[13] iounit[4]` | Make something new under the path; conventionally answered with a stat record of what was made. 9P creates and opens it, and the fid then names it. |
| `remove` → `ok done` | `Tremove fid[4]` → `Rremove` | Remove what is at the path. 9P clunks the fid as well, whether or not the remove worked. |
| `cancel(ticket)` → the stream's last answer, `ok done` | `Tflush oldtag[2]` → `Rflush` | Give up on a request made earlier; answers after it are dropped. |
| any request with the `operator` flag | none; who is asking is settled at `Tattach` | A request from whoever runs the image (see [Operator requests](#operator-requests)). |
| `error errno[4] code[s] description[s]` | `Rerror ename[s]` | The request failed: see [Errors](#errors). 9P's error is a free-form string. |
| a server calls `forward(req, path)`; the caller gets no message for it | none; a proxy makes its own request and copies the answers back | Hand a request being served to a path in the server's own view. The answers go straight to the original caller. |
| none yet | `Tversion`, `Tauth` | Agree on a version and a message size; prove who the user is. |

### Safe verbs

`read`, `watch`, `list` and `stat` are **safe**: they ask about something
without changing it. `write`, `create` and `remove` are not. A read-only
mount answers every unsafe request with `read_only`, and the server never
sees it.

Safety is the only property of a verb that the runtime acts on. Nothing makes
a server honour it, any more than anything stops an HTTP GET from deleting a
record; a server that changes something when it handles a `read` is simply
wrong.

### Streams

Any request can be answered several times. Each answer says whether it is
the last (`done`), and the last may carry no payload. `watch` and `list` are
always streams; a header marks any other answer that is one with `*`. There
is no other streaming or subscription mechanism.

A caller can `cancel` a request at any time. The server is told, if it
wants to know, and the stream ends for the caller at once.

### Operator requests

The `operator` flag marks a request made by whoever runs the image: its
startup requests, and requests an image makes of the instances inside it.
Objects have no way to set it: the host call they make requests with
takes no flags. A header marks a request only the operator may make with
`operator`, as in `/hosts/{name}  operator write text/plain`, and the
runtime answers anyone else `forbidden`, whether or not checks are on, as
it refuses an unsafe verb through a read-only mount.

That is how an object leaves a path to whoever runs it, and how the
runtime's own objects are configured: an operator `write` of their root.

### Errors

A failed request is answered with an error, and that is its last answer.

| errno | code | Meaning |
|---|---|---|
| 1 | `not_found` | nothing is at the path, or the sender's view doesn't reach it |
| 2 | `read_only` | an unsafe verb through a read-only mount, or of something that can't change |
| 3 | `not_supported` | something is at the path, but it doesn't take that verb |
| 4 | `is_directory` | a directory where something else was needed |
| 5 | `not_directory` | something else where a directory was needed |
| 6 | `exists` | `create` of something already there |
| 7 | `invalid` | a malformed path, or a payload the server couldn't make sense of |
| 8 | `io` | the outside world failed |
| 9 | `forbidden` | a request only the operator may make, from an object |
| 10 | `unsupported_type` | the server doesn't take payloads of that content type |

`errno` is what a program compares. `code` is its name, and must be the name
the table gives it, so a log or another transport can be read without the
table. `description` is free text for people, such as why a test failed.

The runtime itself answers `not_found`, `not_supported`, `read_only`,
`invalid` (for a malformed path) and `forbidden`. Everything else, including
`unsupported_type`, comes from servers.

### Stat records

`stat` answers one stat record, `list` streams one per entry, and `create`
conventionally answers one. A stat record is a payload of the content type
`application/x-othismo-stat`, which a header can write as `Stat`. Its bytes
are, little-endian:

```text
name[s] kind[1] length[8] type[s] version[8]
```

- `name`: the last segment of its path.
- `kind`: 0 for something read whole, 1 for a directory.
- `length`: in bytes, or 0 for a directory or if it isn't known.
- `type`: the content type a `read` would answer with, or empty.
- `version`: changes whenever what a `read` would answer changes, or 0 if
  the server doesn't keep track. It is what an HTTP ETag would be made from.

This is the one payload layout the protocol defines. 9P's own stat carries
much more, such as owners, permissions and times; Othismo has no users, and
leaves times to the objects that keep them.

### Forward

A server can hand a request it is serving to a path in its own view. The
runtime resolves the path as if the server had sent the request itself
(through its view, read-only mounts and its `needs`), and delivers it there
with the original caller's ticket. The answers go straight to that caller;
the server that forwarded never sees them.

A proxy could do the same by making its own request and copying every answer
back, which is what a 9P proxy has to do. `forward` saves the copying and
the bookkeeping, for streams and cancels too, and makes a promise the other
way can't: StaticSite never sees the bytes it serves.

### What the runtime checks

On every request it checks the path and verb against the sender's `needs`
and the receiver's `serves` (see [Objects](objects.md#enforcement)), the
read-only rule and the operator rule, and that only a stream gets more than
one answer. It never checks a payload. The checker compares content types,
and message types where headers name them, between needs and what is
mounted when an image is built, and servers answer `unsupported_type` for
anything they don't take.

## Layer 2: payload formats

A payload is a content type and some bytes. The content type says which
**format** the bytes are in; the format, and nothing else, says what is
inside them, including which of the format's own types they hold.

### Content types

Content types are media types, as in HTTP: `text/plain`, `text/html`,
`application/octet-stream`, `application/bson`. A server that serves files
answers each with the type its name suggests; an object that exchanges BSON
messages answers `application/bson`, whatever message it is.

In headers, `*/*` stands for any payload and `text/*` for any text. A server
that takes `write */*` meets a need to write `text/plain`; a need to
`read -> text/*` isn't met by a server that only promises `read -> */*`.
Types and subtypes compare without regard to case. A parameter a header
writes, such as `charset=utf-8`, must be present and equal; one it leaves
out constrains nothing.

### Formats

A format is identified by its media type, and supplies everything Othismo
does with payloads of that type. It does all of it when a module or an image
is built; the runtime uses none of it.

| A format supplies | Used by |
|---|---|
| a schema language, and a parser for it | everything below |
| a rule for when two schemas of one type agree | the checker, when two modules declare the same type |
| an encoder for a literal written in an image or a slot | the image builder |
| a bindings generator for each language | `othismo bindings` |
| documentation rendered from its schemas | `othismo doc` |

Text needs no format: `text/*` payloads are their bytes, in UTF-8.

### BSON

`application/bson` is the first format, and the one the AssemblyScript
library supports. A payload is one BSON document whose first key, `$type`,
names its message type; the remaining keys are its fields. `$type` is the
wire's: images, value files and `--set` name the type before the braces,
`Greeting { ... }`, and the format writes it. Its schemas declare message
types, and enums for their fields:

```text
export schemas application/bson {
  ## Configures a Greeter; sent once at startup by the image.
  message Greeting {
    ## Put before each name.
    prefix: string
    ## End with `!` rather than `.`.
    excited: bool
  }
}
```

Fields may be other messages, lists, maps from strings, enums and `any`
message, and may be optional:

```text
schemas application/bson {
  message Party {
    host: Friend
    guests: [Friend]
    ## Optional: a party may leave it out.
    style?: Style
    seats: {u8}
  }

  message Friend { name: string, since: time, id: uuid }

  ## How to greet.
  enum Style { plain, shouted }
}
```

| Field type | BSON | AssemblyScript |
|---|---|---|
| `string` | string | `string` |
| `bool` | boolean | `bool` |
| `i8`, `i16`, `i32`, `u8`, `u16` | int32 | the same |
| `i64`, `u32`, `u64` | int64 | the same |
| `f32`, `f64` | double | the same |
| `bytes` | binary (generic subtype) | `ArrayBuffer` |
| `time` | UTC datetime, to the millisecond | `Date` |
| `uuid` | binary (UUID subtype) | `Uuid` |
| a message type, `Friend` | document, without `$type` | `Friend` |
| an enum, `Style` | string, the value's name | `Style` |
| `any` | document, with its `$type` first | `Message` |
| `[T]`, a list | array | `Array<T>` |
| `{T}`, a map from strings | document | `Map<string, T>` |

A field written `name?: T` is optional: a value may leave it out, and so
does the wire, and a reader takes a null as left out too. In AssemblyScript
it may be null, so one whose type can't be, such as `i32` or an enum, is a
`Box<i32> | null`. A message field's type is known, so it carries no
`$type`; an `any` field holds a message of whatever type, which says its own.
A type may be named before it is declared, and importing a type brings the
types its fields name.

Literals write these as you would expect: `[ ... ]` for a list, `{ key:
value }` for a map, quoting keys that aren't names, a string for an enum's
value, a time in RFC 3339, `"2026-10-23T12:00:00Z"`, or as milliseconds
since 1970, and a uuid as people write one. A message in an `any` field names
its type, `extra: Note { text: "hi" }`.

A message type is its name and its exact fields, and an enum its name and
values. Two modules in one image may both declare `Greeting` if they agree
on it, and can't be used together if they don't; that is the format's rule,
checked when the image is built. An enum is not a payload, so headers can't
name one.

### Where schemas live

A module's schemas are in its header, in a `schemas <content type> { ... }`
block, one per content type. The block's body is in the format's own
language, and Othismo reads only as far as its closing brace, so a format
can write whatever it needs there. A format lists the types a block
declares, and those names are what headers, images and slots use:

```text
/greeting  write Greeting                           # a header
write /greeter/greeting Greeting { prefix: "Hi" }   # an image
require greeting : Greeting                         # a slot
```

A name stands for its content type, `application/bson`, which is all the
runtime ever sees, and for its type, which the checker and codegen use. A
content type with schemas can't be written bare in its place. Types are
imported like any other name, from a file whose block is exported:

```text
import { Greeting } from "../greeter/scripts/Greeter.oth"
```

The schemas are embedded in the compiled module beside its header, one
custom section per format, so a module describes its payloads as well as
its paths. Another module's build can generate bindings for both from the
`.wasm` alone.

## The host ABI

The ABI is Layer 1 as host calls. Paths, content types and descriptions
are UTF-8; a length of 0 for both a type and its bytes means no payload.

Imports, from module `othismo`, plus `env.abort`:

| Import | Meaning |
|---|---|
| `call(verb, path, path_len, type, type_len, body, body_len, expect) -> ticket` | make a request. With `expect` 0, the ticket is 0 and answers are dropped |
| `reply(req, type, type_len, body, body_len, done)` | answer a request being served, successfully |
| `fail(req, errno, desc, desc_len)` | answer it with an error, the last answer |
| `forward(req, path, path_len)` | hand it to a path in our view |
| `cancel(ticket)` | give up on a call; later answers are dropped |
| `bind(src, src_len, at, at_len, flags) -> errno` | |
| `unmount(at, at_len) -> errno` | |
| `expose(at, at_len, src, src_len, flags) -> errno` | |

Exports: `memory`, and

| Export | Meaning |
|---|---|
| `rt_inbox(len) -> ptr` | a buffer for the host to write into |
| `rt_serve(req, verb, flags, path_len, type_len, body_len)` | a request; the inbox holds the path, the type, then the bytes |
| `rt_answer(ticket, errno, type_len, body_len, done)` | an answer. With `errno` 0, the inbox holds the type then the bytes; otherwise it holds the description |
| `rt_start()` | once the image is set up |
| `rt_cancel(req)` | optional. Without it, a cancelled request's answers are dropped until the module ends it |

Request ids are numbered per server, and tickets per caller. Answering an
unknown request id is an error that stops the run.

## The SDK

The AssemblyScript library follows the layers, and generated code joins
them:

| Part | What it has | Where it comes from |
|---|---|---|
| `"othismo"` | `Server`, `Request`, `Response`, `Call`, `Verb`, `Errno`, `Payload`, `Stat`, `ns` for requests and the view, and continuations such as `OnPayload` and `OnText` | the library |
| `"othismo/bson"` | `Message`, `Writer`, `Reader`, `registerMessage`, `bson.payload(msg)` and `bson.decode(payload)`, and `Box` and `Uuid` for fields | the library |
| `"othismo/env"` | clients and server bases for the runtime's traits: `ConsoleClient`, `ListenerClient`, `TestSuiteServer`, ... | generated from `env` |
| `./Greeter.oth` | `GreeterServer`, a client for each of its needs, and `Greeting`, registered with `"othismo/bson"` | generated from `Greeter.oth`, the classes by each type's format |

The SDK is in `sdk/`: the library in `sdk/assemblyscript/`, the runtime's
header in `sdk/env/`, and reusable objects in `sdk/objects/`, such as
`HttpServer` and `StaticSite`, which an image imports by path:
`import { HttpServer } from "../../sdk/objects/http/HttpServer.oth"`.

A `Payload` is a content type and an `ArrayBuffer`. One whose type the
module doesn't understand can be passed on unchanged.

**Generated from the header**, a server base class has a method for each
request its `serves` lists, named for the verb and the path's literal
segments, with the path's `{name}` segments as arguments. It routes each
request to its method and answers the rest: `not_supported` for a verb
nothing takes, and `unsupported_type` for a payload whose content type the
entry doesn't take. A client has the same methods for the requests a need
lists, and is made with the mount point it goes through. A need for one
mount point gets one made already, named for the mount point's last segment:
`cons` for `needs /dev/cons`. A need under a pattern, such as `needs /**`,
gets only the class, to make with whichever mount point the module is told
to use. Where an entry
names one text type, such as `text/plain`, the methods take and give a
`string`; where it names a message type, that message, decoded for a
server and encoded for a client; otherwise a `Payload`. A verb that takes
several message types has a method for each, and anything else is answered
`invalid`.

**Generated from the schemas**, each message type is a class with its
fields, encoders and decoders, generated by its format, for the types the
file declares and those its objects take or answer. Every format's classes
have `static fromPayload(p): T | null` and `toPayload()`, which the server
bases and clients use. For BSON, `bson.decode(payload)` gives a `Message`,
or an `Unknown` message for a type the module has no class for, which is
passed on unchanged, in an `any` field too. Each enum is an AssemblyScript
enum, with `nameOf<Enum>(v)` for a value's name. A reader skips fields it
doesn't ask for, whatever their BSON type.

So a module that uses another gets, from that module's `.wasm`, the paths
and verbs it can call, and the messages each one takes, as methods and
classes its compiler checks, without reading anyone's guide to find out
what `/greeting` takes.

```ts
import { Request, OnText, run } from "othismo"
import { Greeting, GreeterServer, cons, shouter } from "./Greeter.oth"

class Greeter extends GreeterServer {
  prefix: string = "Hi, "
  excited: bool = false

  start(): void {
    cons.watch(new OnName())                    // watch / -> text/plain*
  }

  writeGreeting(req: Request, g: Greeting): void {    // write /greeting Greeting
    this.prefix = g.prefix
    this.excited = g.excited
    req.end()
  }

  greet(name: string): void {
    const greeting = this.prefix + name + (this.excited ? "!" : ".")
    cons.write(greeting, null)                  // write / text/plain
    shouter.write(greeting, null)
  }
}

const greeter = new Greeter()

class OnName extends OnText {
  text(line: string, done: bool): void { greeter.greet(line) }
}

run(greeter)
```

## Not yet

- `version` and `auth`.
- Anything like `wstat`.
- Union mounts, and a `/srv` convention for posting a served tree.
- Timeouts on calls.
- Formats other than BSON. A TypeSpec emitter would be one way to write
  schemas for one.
