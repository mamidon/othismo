# othismo

This is the runtime for **Othismo**: programs built from sandboxed objects
that can reach only what a short, declarative image gives them, so you can
review the image and the interfaces instead of every line of the code. See
[the overview](docs/overview.md) for what it is and why.

```sh
task run:example:static   # serve the docs at http://127.0.0.1:8080 (needs Rust, Node and Task)
task test
task --list                # everything else
```
