# Messaging Interface & PIDs

> **Implementation status (2026-05-09).** This document is mostly a forward-looking
> design. Roughly:
>
> - **Implemented:** BSON message envelope with optional `othismo.{send_to, reply_to, response_id}`
>   header (`sdk/src/lib.rs`); routing by `send_to` through `NamespaceRouter`
>   (`othismo/src/othismo/namespace.rs`); `_send_message` host syscall;
>   `_allocate_message` and `_message_received` guest exports; CLI commands for
>   `new-image`, `import-module`, `remove-module`, `instantiate-instance`,
>   `delete-instance`, `send-message` (empty payload only), `list-objects`.
> - **Not implemented:** `_cast_message`, `_othismo_start`; any built-in
>   `othismo.namespace.*` / `othismo.http.*` / `othismo.error` message types;
>   native modules (`othismo.console`, `othismo.namespace`, `othismo.http`,
>   `othismo.blobs` — only a stub trait in `native_modules/mod.rs`); symlinks,
>   mounts; CLI message-content / templating; `reply_to` / `response_id`
>   correlation through the router. Inline `[STATUS]` tags below mark each item.

When configuring an image; we’re importing modules & instantiating instances.  Somehow we have to wire up these
Instances such that they can actually interact.

~~Option A: Direct configuration via messaging instances.  (Need to bind instances to names or IDs)~~

Option B: Massive manipulation of the namespace, ala Plan9.


### All Information Inside Messages

Instead of having multiple parameters, we encode everything inside the message via top level namespaces.  e.g.

```
{
  "othismo.send_to": "/namespace/foo",
  "acme.custom_message": {
    "foo": "bar"
  }
}
```


## Syscalls

The boundary ended up split between **host imports** (functions Othismo provides
to the guest under the `othismo` module) and **guest exports** (functions the
host calls on the instance).

### _send_message(bytes: *mut u8, length: u32) -> u32  *[host import — IMPLEMENTED]*
Guest tells the host a message has been placed in the guest's memory at the
given pointer/length. The host copies the bytes out and forwards the message
to the `NamespaceRouter` outbox. Returns a handle (currently the buffer pointer
cast to `u32`). The original design said responses would arrive with this handle
in their `request_handle`; correlation isn't wired up yet.
Implementation: `othismo/src/othismo/executors.rs` — `native_trampolines::send_message`.

### _allocate_message(message_length: u32) -> *const u8  *[guest export — IMPLEMENTED]*
**This is a guest export, not a host syscall.** The host calls it to ask the
guest for a buffer in linear memory of the requested size; the host then writes
the message bytes into that buffer and follows with `_message_received`. The
earlier design had a richer signature `(handle, length, request_handle)` —
that was simplified down: the buffer pointer itself is the handle.
Implementation: `prototype/src/abi.rs` (guest), `executors.rs::receive_message` (host caller).

### _message_received(message_handle: u32) -> ()  *[guest export — IMPLEMENTED]*
Guest export the host calls after writing into the buffer returned by
`_allocate_message`. The guest takes the buffer out of its inbox and spawns a
task on its internal executor to process it. (This replaces the
`_process_message` name used in the older `runtime.md` sketch.)
Implementation: `prototype/src/abi.rs`.

### _run() -> ()  *[NOT a host call — guest-internal]*
The original design had this as a host-driven message pump. In practice the
host doesn't call it; messages are processed synchronously inside
`_message_received`, and `_run` only exists in the guest as a helper that
calls `executor().try_tick()` until idle. Used by the prototype's tests and
not by the host.
Implementation: `prototype/src/abi.rs`.

### _cast_message(bytes: *const u8, length: u32) -> u32  *[NOT IMPLEMENTED]*
Designed as a fire-and-forget variant of `_send_message`. Not implemented; in
the meantime, all `_send_message` calls are effectively fire-and-forget because
response routing isn't wired up.

### _othismo_start() -> ()  *[NOT IMPLEMENTED]*
Init hook for instances. Not implemented; instances are loaded from the image
and start consuming messages immediately when the namespace boots.

## Othismo built in messages for HTTP

> Status note: only the `othismo` envelope (`send_to`, `reply_to`, `response_id`)
> is implemented in `sdk/src/lib.rs`, and only `send_to` actually drives routing
> in `NamespaceRouter`. Everything below is design-only unless tagged otherwise.

### othismo  *[PARTIALLY IMPLEMENTED]*
This message is included alongside other messages for specific routing.

Currently `send_to` is honored by the router; `reply_to` and `response_id` are
defined on the struct but not yet used during dispatch.

```
{
    "othismo": {
        "send_to": "/some/thing",
        "reply_to": "/some/other/thing" // optional, to redirect responses elsewhere
    },
    "acme.custom_message": {
        // this will be sent to /some/thing
    }
}
```

### othismo.namespace.instantiate  *[NOT IMPLEMENTED — CLI only via `instantiate-instance`]*
Instantiates a new instance from a module in the image.

```
{
    "othismo.instantiate": {
        "module": "/some/namespace", // the module 
        "name": "/some/other/namespace" // location in the namespace of the new instance
    }
}
```

### othismo.namespace.import  *[CLI only — IMPLEMENTED via `import-module`; no message form]*
Only available via the CLI.  Imports a webassembly module into the namespace.
By default, ./foo/fizzbuzz.wasm is imported to /othismo/modules/foo/fizzbuzz.
Providing `name` changes this destination.

(Current CLI: `othismo <image> import-module <path>` — derives the namespace
name from the file stem; no `name` override yet, no `/othismo/modules/` prefix.)

```
{
    "othismo.import": {
        "file": "fizzbuzz.wasm", // the relative path of the file, including extension 
        "name": "/othismo/modules/fizzbuzz" // optional
    }
}
```

### othismo.namespace.list  *[NOT IMPLEMENTED — CLI only via `list-objects`]*
Lists out all items in the namespace.  If `prefix` is provided,
the output is filtered by that prefix.

(Current CLI `list-objects` lists all objects with no prefix filter.)

```
{
    "othismo.namespace.list": {
        "prefix": "/some/namespace/prefix" // optional
    }
}
```
The response is:
```
{
    "othismo.namespace.list.response": [
        "/something/in/the/namespace",
        "/some/other/thing"
    ]
}
```

### othismo.namespace.make_path  *[NOT IMPLEMENTED]*
Create directories required for a path.
Directory names cannot conflict with existing objects.
```
{
    "othismo.namespace.make_path": {
        "path": "/some/fully/qualified/path"
    }
}
```

### othismo.namespace.sym_link  *[NOT IMPLEMENTED]*
TODO -- redirects all messages at a particular path to another path
### othismo.namespace.mount  *[NOT IMPLEMENTED]*
TODO -- redirects all namespace operations at or below /some/path to a particular instance
TODO -- how are mount & sym links different.. are they?

### othismo.http.request  *[NOT IMPLEMENTED]*
Represents a received HTTP request.
```
{
    "othismo.http.request": {
        "host": "your_domain.com",
        "method": "GET" // POST etc
        "endpoint": "/some/relative/path",
        "query": {
            "key": "value"
        },
        "headers": {
            "key": "value"
        },
        "body": ... // bson bytes
    }
}
```


### othismo.http.request.response  *[NOT IMPLEMENTED]*
Represents a response to a previous HTTP request.

```
{
    "othismo.http.request.response": {
        "status" 200,
        "headers": {
            "key": "value"
        },
        "body": ... // bson bytes
    }
}
```

### othismo.error  *[NOT IMPLEMENTED]*
Anything can respond with an error
```
{
    "othismo.error": {
        "code": "unique_code",
        "message": "A human friendly message"
    }
}
```


## Configuring the server, with files

> **Aspirational.** None of this works end-to-end yet. Specifically: native
> modules (`othismo.http`, `othismo.blobs`) don't exist beyond a stub trait;
> `import-module` only loads `.wasm` files from the filesystem; `send-message`
> currently sends an empty BSON document and accepts no payload args; `mount`
> and `sym-link` are not CLI commands.
>
> Working CLI today: `new-image`, `import-module <wasm-path>`, `remove-module`,
> `instantiate-instance <module> <name>`, `delete-instance`,
> `send-message <name>` (empty payload), `list-objects`.

```
othismo new-image image
# othismo always exists in the namespace at /othismo
othismo image import-module othismo.http
othismo image import-module othismo.blobs

# import the actual custom code, which maps HTTP requests to blob responses
othismo image import-module prototype

# /modules contains othismo.http, othismo.blobs, prototype


# instantiate a web server
# also creates a folder /server/sites/ & /server/content/ where handlers or content for requests are found, hopefully
# also creates a folder /controller/content/ where 
othismo image instantiate-instance othismo.http server
othismo image instantiate-instance prototype controller
othismo image instantiate-instance othismo.blobs content

# import all files in ./www/ into blob storage
othismo image send-message /content cp=./www/
othismo image mount /content /server/content/othismo.com

# requests to the host othismo.com will be handled by /
othismo image sym-link /controller /server.sites/othismo.com 
```