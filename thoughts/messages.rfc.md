# Messages — Format & Semantics

> **Status (2026-05-23):** Design-only. This document describes the target
> wire format. The SDK and router currently implement an older shape — a
> top-level `othismo` envelope with `send_to` / `reply_to` / `response_id`
> fields — see `sdk/src/lib.rs` and `othismo/src/othismo/namespace.rs`.
> Migration to the shape below has not started, and every named message
> type in §5 is also unimplemented.

This document describes the wire format and semantics of messages
exchanged between instances and the Othismo host. How those messages flow
through guest memory and the async runtime is covered in `runtime.md`. How
addresses resolve to instances is covered in `namespace.md`.

## 1. Encoding

A message is a single BSON document.

Messages cross the host/guest boundary as a contiguous byte buffer. The
host calls the guest export `_allocate_message(length)` to reserve space
inside the guest's linear memory, writes the BSON bytes into that buffer,
and then calls `_message_received(handle)` to notify the guest. Outgoing
messages go the other way: the guest places BSON bytes in its own memory
and calls the host import `_send_message(ptr, length)`; the host copies
them out and forwards to the `NamespaceRouter`. See `runtime.md` for the
full sequencing.

All addressing and metadata lives inside the BSON document — there is no
separate header on the wire.

## 2. Message shape

A message has a single top-level key of the form `/path.operation`,
whose value is the operation's parameter document.

```bson
{
  "/foo/some_instance.read": { "path": "/x" }
}
```

- `/foo/some_instance` is the namespace path of the recipient instance.
  Path segments use `/` as a separator; segments themselves cannot
  contain `.` (see `namespace.md`).
- `.read` is the name of the operation to invoke on that instance.
- The value document holds the parameters the operation expects. Parameter
  keys are defined by the operation's contract.

The wire format does not technically prevent multiple top-level keys per
message, but the convention is **one operation per message**. Batching is
deferred until there is a concrete need.

## 3. Cross-cutting metadata convention

Cross-cutting concerns — routing, telemetry, deadlines, etc. — are
encoded as `/`-prefixed sub-keys inside the operation's parameter
document. By convention they are grouped under labels that borrow the
`/path.operation` shape:

```bson
{
  "/foo/some_instance.read": {
    "path": "/x",
    "/othismo.routing":   { "reply_to": "/bar/handler.write", "response_id": 42 },
    "/othismo.telemetry": { "trace_id": "abc", "parent_span": "def" }
  }
}
```

This is **purely a naming convention**. No resource named `/othismo`
exists in the namespace, and no `routing` operation is ever dispatched.
The shape is borrowed because it gives cross-cutting metadata a
collision-free, visually distinct namespace that sits cleanly next to
operation-defined parameters. A recipient is free to read these keys,
ignore them, or treat them as opaque bytes — nothing about message
dispatch depends on their presence.

### 3.1 Suggested starting categories

- **`/othismo.routing`** — fields the runtime and recipients use to
  correlate responses:
    - `reply_to: String` — namespace path the recipient should address
      its response to, if any.
    - `response_id: u64` — sender-chosen correlation handle; the
      recipient echoes this value inside the response's own
      `/othismo.routing` block.
- **`/othismo.telemetry`** — fields propagating tracing context across
  message hops. Field set is TBD; expect `trace_id`, `span_id`,
  `parent_span`, and possibly baggage.

Both categories are optional. New cross-cutting concerns can be added as
new namespaced sub-keys without changing the rest of the shape.

## 4. Routing semantics

1. The router reads the message's top-level key (e.g.
   `/foo/some_instance.read`).
2. It splits on the last `.` to recover the recipient path
   (`/foo/some_instance`) and the operation name (`read`).
3. It looks up the instance at that path and delivers the operation name
   and parameter document to that instance's inbox.
4. If the recipient produces a response, it sends a new message whose
   top-level key is the path/operation given by the original
   `/othismo.routing.reply_to`, with the original `response_id` inside
   its own `/othismo.routing`. The requesting runtime uses the id to
   correlate the response with the in-flight request.

How a path resolves to an instance — exact match, mount tables, sym
links — is described in `namespace.md`.

## 5. Standard message types

The types below are the messages Othismo expects to define out of the
box. All are design-only.

### 5.1 `/othismo/namespace.instantiate`

Instantiate an imported module at a given path.

```bson
{
  "/othismo/namespace.instantiate": {
    "module": "/othismo/modules/fizzbuzz",
    "name":   "/some/path/instance_name"
  }
}
```

CLI equivalent today: `instantiate-instance <module> <name>`.

### 5.2 `/othismo/namespace.import`

Import a `.wasm` module from the host filesystem into the image.

```bson
{
  "/othismo/namespace.import": {
    "file": "fizzbuzz.wasm",
    "name": "/othismo/modules/fizzbuzz"
  }
}
```

CLI equivalent today: `import-module <wasm-path>` (derives the namespace
name from the file stem; no `/othismo/modules/` prefix yet).

### 5.3 `/othismo/namespace.list`

Enumerate paths in the namespace, optionally filtered by prefix.

```bson
{
  "/othismo/namespace.list": {
    "prefix": "/some/namespace/prefix",
    "/othismo.routing": { "reply_to": "/caller.list_response", "response_id": 7 }
  }
}
```

The response is addressed back to the caller:

```bson
{
  "/caller.list_response": {
    "entries": [
      "/something/in/the/namespace",
      "/some/other/thing"
    ],
    "/othismo.routing": { "response_id": 7 }
  }
}
```

CLI equivalent today: `list-objects`.

### 5.4 `/othismo/namespace.make_path`

Create the intermediate directories required by a fully-qualified path.
Directory names cannot conflict with existing objects.

```bson
{
  "/othismo/namespace.make_path": {
    "path": "/some/fully/qualified/path"
  }
}
```

### 5.5 `/othismo/namespace.sym_link`

Redirect all messages addressed to one path to another. Semantics
overlap with `mount`; the distinction is still TBD.

### 5.6 `/othismo/namespace.mount`

Delegate the namespace sub-tree at or below a given path to a particular
instance.

### 5.7 HTTP request delivery

When a future native HTTP module receives a request, it forwards it to
the resolved handler instance using the operation name `http_request`:

```bson
{
  "/sites/acme_com.http_request": {
    "host":     "acme.com",
    "method":   "GET",
    "endpoint": "/some/relative/path",
    "query":    { "key": "value" },
    "headers":  { "key": "value" },
    "body":     /* bson bytes */,
    "/othismo.routing": {
      "reply_to":    "/net/http/web_http.deliver_response",
      "response_id": 42
    }
  }
}
```

The handler replies by addressing the `reply_to`:

```bson
{
  "/net/http/web_http.deliver_response": {
    "status":  200,
    "headers": { "key": "value" },
    "body":    /* bson bytes */,
    "/othismo.routing": { "response_id": 42 }
  }
}
```

Operation names like `http_request` and `deliver_response` are
conventions for instances that want to participate in HTTP. There is no
global registry yet — participants just have to agree on the shape.

### 5.8 Errors

Any instance may respond with a structured error in place of a normal
response. The shape is TBD; expect at least a machine-readable code and a
human-friendly message.

## 6. Open questions

- **Response correlation.** The runtime needs an in-flight request table
  keyed by `response_id`, populated by `_send_message` and drained when
  the router sees a matching response. None of this exists yet.
- **Fire-and-forget.** `runtime.md` describes `_cast_message` as a
  fire-and-forget variant of `_send_message`; it is not implemented.
  Today every send is effectively fire-and-forget because responses are
  not routed.
- **Missing-recipient semantics.** The router currently panics if no
  process matches the destination. This likely needs to become a
  structured error returned to the requester when the requester can be
  identified.
- **Multi-op messages.** The wire format permits multiple top-level
  `/path.operation` keys; the convention is one per message. If batching
  is ever introduced, semantics (atomic? parallel? ordered?) need to be
  specified.
- **Two-doc wire framing.** Splitting the cross-cutting metadata back
  out of the body into a separate envelope BSON document on the wire
  (envelope + body) is a possible upgrade path if router cost ever
  matters at scale. The body shape — `{ "/path.op": params }` — would
  not need to change.
