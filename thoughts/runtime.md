> **Implementation status (2026-05-09):** the architecture sketched below largely
> landed in code. Host runs on a multi-threaded tokio runtime (`othismo/src/othismo/namespace.rs`);
> each instance is a tokio task driving a wasmer `Instance`
> (`othismo/src/othismo/executors.rs`). A `NamespaceRouter` owns the dispatch
> mpsc channel and routes messages to per-process inboxes. Inside the guest,
> the prototype SDK uses a single-threaded `async_executor::StaticLocalExecutor`
> (`prototype/src/tasks.rs`). The host does *not* call `_run` — instead it
> calls `_message_received` synchronously per message, and the guest spawns a
> task per message on its own executor. The exact syscall names below differ
> from what shipped — see the inline notes.

So far, I've been building Othismo & instances as a single threaded system.
But that's just not going to work once we consider the likelyhood that messages & responses will be
both re-entrant to the source instance and inter-leaved with other communications.

This implies using async inside of the instances, but what about Othismo itself?

I ASS-U-ME using Futures instead of threads is better, as we may end up with a lot of instances in a given environment.

So... what should the unit of work be?  I suppose not the instance itself, as that basically devolves into a message loop inside of a message loop of whatever executor I'm using.
And instances can be idle...

Would the processing of a message itself make a good Future?

Supposing a fresh environment receives a message from outside...

1. Message A is sent to Instance A
    2. A buffer to store the message is allocated in Othismo
    3. The recipient Instance is located in the Namespace
    4. The Instance provides a destination buffer
    5. The message is copied into the Instance's linear memory
    6. Invoking _message_received on the Instance
    7. _message_received might return immediately indicating no response
    8. ... or indicating a response may be pending
    9. ... the future waits for the response, if any


Meanwhile inside the instance is a whole other executor handling the criss cross of messages it cares about.
This means the references Othismo holds to the instances must be protected, since WASM is actually single threaded.
But it also means the inner executor is only ever doing 1 thing at a time and I *think* it never actually needs to poll it's futures.
Since the inner futures should all be sent messages awaiting a response, and for the purposes of timing out I suppose we can let
the host handle that.

This implies the following "syscalls", none of which are blocking:

_send_message(address_of_bytes, length_of_bytes) -> handle
Othismo copies the message out of guest memory and forwards it to the NamespaceRouter via the outbox channel.
(Implemented in `executors.rs` — `native_trampolines::send_message`. The handle is currently the buffer
pointer cast to u32; correlation IDs for response routing aren't wired up yet.)

_allocate_message(message_length) -> address
Guest export. Host calls it to reserve a buffer in the guest's linear memory, then writes the
message bytes there. (Implemented as a `_allocate_message` *export* in `prototype/src/abi.rs`,
called from `InstanceTask::receive_message` on the host.)

_message_received(handle) -> ()
Guest export. Host calls it after `_allocate_message` + memory write to tell the guest a message
is in its inbox; the guest's executor spawns a task to process it. (This took the place of
the `_process_message` name used earlier in this sketch.)

Supposing we had an Othismo environment with instance Router and instance Echo which receives a
web request.  Router forwards that request to Echo, which simply echos it back.

Individual Futures are indicated by indentation.

External Caller    Othismo            Router             Echo
|                |                 |                 |
|---M1---------> |                 |                 |  Web request (M1)
|                |---M1--------->  |                 |  Othismo forwards M1 to Router
|                |  _allocate_message(len)            |  Host reserves buffer in Router
|                |  _message_received(M1_handle)      |  Router processes M1
|                | <-_send_message(M2_bytes, len)     |  Router sends M2 to Othismo outbox
|                |                 |----M2------->   |  NamespaceRouter delivers M2 to Echo
|                |                 | _allocate_message(len)         Host reserves buffer in Echo
|                |                 | _message_received(M2_handle)   Echo processes M2
|                |                 | <-_send_message(M3_bytes, len) Echo sends response M3
|                |<----M3--------|                 |  Othismo receives M3
|<---M3---------|                 |                 |  Othismo sends M3 to External Caller
|                |                 |                 |

Note: the design above assumes responses can be correlated back to the request that triggered
them. The current implementation has a `response_id` field on the `othismo` envelope
(`sdk/src/lib.rs`) but does not yet populate or route on it — outgoing messages from a guest
are sent fire-and-forget through the router based on `othismo.send_to`.
