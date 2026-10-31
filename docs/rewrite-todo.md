# Docs rewrite

The docs are being rewritten by hand, for people: dictated a section at a
time, then shaped into prose. The current pages in `docs/` stay as reference
until their replacements are done.

Each section: [ ] dictated, then [ ] drafted, then [ ] reviewed.

## 1. Overview (the landing page)

One screen. Done: `docs/overview.md`, its image copied into `examples/static/site.oth`.

- [x] What it is, in one sentence
- [x] Why, in two or three sentences
- [x] The image serving this site (`examples/static/site.oth`), unchanged
- [x] The promise: nothing reaches what it wasn't given, host included
- [x] Link to the tour, then the other main pages (no `--check` output)

## 2. A tour by example

The static site, file by file, each walked through in the order of its lines.
Drafted in `docs/tour.md`.

- [x] `site.oth`: needs, the three instances, `mount /http`, the hosts, `export`
- [x] `site.oth`: the imports
- [x] `static.oth`: running it for real, and its needs
- [x] HttpServer's header: what you read instead of its code

## 3. Objects

- [ ] What an object is: a WebAssembly module in a sandbox
- [ ] No ambient access: no files, clock, network or randomness
- [ ] Server and client at once
- [ ] Single-threaded, sharing nothing

## 4. Namespaces

- [ ] Each object's private view of the world (analogy before Plan 9)
- [ ] View and served tree
- [ ] Mounting: how one object appears in another's view
- [ ] Names are the capabilities: what you can't see doesn't exist
- [ ] Rearranging a view at run time, and why it can't widen reach
- [ ] Read-only
- [ ] Conventions: `/dev/cons`, `/net/tcp`, `/files`
- [ ] Aside: if you know Plan 9

## 5. Requests

- [ ] A request: path, verb, payload
- [ ] The seven verbs, and safe vs. unsafe
- [ ] Answers: errors, streams, cancel
- [ ] Forward
- [ ] Operator requests
- [ ] Payloads: content types, and the runtime never looking inside
- [ ] Formats, and BSON as the first
- [ ] Naming the layers (decide: "0P" or Layer 0/1/2)
- [ ] Aside: if you know 9P or HTTP

## 6. Interfaces

- [ ] Two kinds of interface: between objects, and between a module and the runtime
- [ ] The header: `serves` and `needs`
- [ ] Protocols and traits
- [ ] Schemas: describing messages
- [ ] Shapes are enforced, behaviour isn't (and how that differs from TypeScript)
- [ ] Checking the wiring when an image is built
- [ ] The ABI: host calls in and out, tickets, one thing at a time
- [ ] Why the ABI is shaped that way: nothing serialized, no blocking calls
- [ ] Codegen: `bindings`, compile, `embed`; a module that describes itself
- [ ] Languages: why AssemblyScript, and what another language would need
- [ ] Documentation in headers: `##` comments and guides

## 7. Images

- [ ] What an image is, and why a file you can read (Plan 9 vs. Docker)
- [ ] Instances, `mount` and `bind`
- [ ] Startup requests
- [ ] Images inside images: `export`, private instances, operator of its own
- [ ] A starting point, not a saved state (vs. Smalltalk), and what bounds it

## 8. The edge

- [ ] Environment objects: the only way out
- [ ] `needs`, and only the root's being filled for real
- [ ] "What can this program do to the machine?" answered in one place
- [ ] `--check`: reach computed without running anything

## 9. Building, deploying and testing

- [ ] Slots: values from whoever runs the image
- [ ] Building a `.rti`, and what it holds
- [ ] Running and deploying
- [ ] Packed files vs. Files
- [ ] Testing: test suites and in-memory stand-ins
- [ ] The shell (brief, pointing at Open questions)

## 10. Not yet: designed, not built

- [ ] Auth: different consumers seeing different subsets of a tree
- [ ] Union mounts, and `/srv`
- [ ] Call timeouts
- [ ] Containing a failed object
- [ ] CPU and memory limits

## 11. Open questions: not thought through yet

- [ ] Configuration: slots, `--set` and value files work, but no coherent story
- [ ] The standard library: what environment objects and traits should be
- [ ] Remote connections
- [ ] The shell
- [ ] Telemetry
- [ ] The image language: dropping `bind` for `mount` everywhere, dropping
      `readonly` for auth, better syntax for `needs` and `export` (noted in
      the tour)

## 12. Where it comes from

- [ ] History: the iterations, and starting over top-down
- [ ] Black boxes and object orientation as Smalltalk meant it
- [ ] Plan 9
- [ ] Docker
- [ ] Smalltalk
- [ ] HTTP
