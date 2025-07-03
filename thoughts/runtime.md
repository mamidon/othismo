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

_send_message(address_of_bytes, length_of_bytes) -> bytes_sent
Othismo copies the message into it's memory, and creates a Future to process it.

_allocate_message(message_length) -> address
Instance allocates a buffer for the message to be received into.

_process_message(address) -> outcome
Creates an inner Future to process the message.  Executes immediately until the first `_send_message`.
The return result either indicates that processing is complete, that a response message was generated, or a response is expected at some point later.

Supposing we had an Othismo environment with instance Router and instance Echo which receives a
web request.  Router forwards that request to Echo, which simply echos it back.

Individual Futures are indicated by indentation.

External Caller    Othismo            Router             Echo
|                |                 |                 |
|---M1---------> |                 |                 |  Web request (M1)
|                |---M1--------->  |                 |  Othismo forwards M1 to Router
|                                  | _process_message(M0, M1)          |  Router processes M1
|                | _allocate_message(len)            |  Router allocates buffer for M2
|                | _send_message(M1, M2, ...)        |  Router sends M2 to Echo
|                |                 |----M2------->   |  Othismo delivers M2 to Echo
|                |                                   | _process_message(M1, M2)       Echo processes M2
|                |                 | _allocate_message(len)         Echo allocates buffer for M3
|                |                 | _send_message(M2, M3, ...)     Echo sends response M3
|                |<----M3--------|                 |  Othismo receives M3
|<---M3---------|                 |                 |  Othismo sends M3 to External Caller
|                |                 |                 |
