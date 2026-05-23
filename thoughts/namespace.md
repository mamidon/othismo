> **Status (2026-05-23):** Largely aspirational. A namespace implementation
> exists today but is very limited — see `web_server.md` for per-operation
> status notes. This document describes the design we're building toward,
> not what ships.

The Namespace is the central abstraction that enables all IPC in Othismo.
Its goal is to mediate access between instances and to enable introspection
and manipulation of the runtime environment.

How messages are encoded and the routing fields they carry is covered in
`runtime.md`. This document is about how those messages are routed and the
primitives we will build a functional system out of.

## The simplest paradigm

The simplest paradigm is for every object to have a path, like files do
today. You just need to know what message(s) an instance will respond to
and send to that path.

This works, and with some conventions around naming it can probably go
quite far. But it means introspection is limited.

Supposing an instance controls the namespace underneath it, it could make
itself much more easily introspectable. Some notation may be needed to mark
where this cutover happens, but let's avoid that for now.

## Naming convention

Instances are named `NAME.MODULE_NAME` and live at a path. Instantiating
module `MODULE_NAME` with the name `NAME` under `/some/path/` produces the
default instance path `/some/path/NAME.MODULE_NAME`.

## Example: HTTP server with two backends

A website with an HTTP server instance and two different backend
applications might construct a namespace like this.

`/othismo/namespace` is the instance that controls namespace operations:

```
/othismo/namespace
    /import      // import a .wasm module from the host system into /othismo/modules
    /sym_link    // sym_link /a /b — all operations directed to /a go to /b
    /mount       // mount an instance to handle some sub-tree of the namespace
```

`/othismo/modules` is the directory where imported modules are represented
in the namespace. Every module exposes an `/instantiate` sub-path; sending
to it instantiates the module at `/some/path/instance_name`.

```
/othismo/modules/
    http/instantiate
    app/instantiate
    file_content/instantiate
```

`/net/http/web.http/` is an instance of `http`; it sym links
`./sites/www.othismo.com` and `./sites/www.mamidon.com` to backend
instances:

```
/net/http/web.http/sites/www.othismo.com -> /bin/othismo.app/www
/net/http/web.http/sites/www.mamidon.com -> /bin/personal.file_content
```

`/bin/othismo.app` and `/bin/personal.file_content` are instances of their
respective modules. Each mounts various sub-paths it handles.
`othismo.app` is an actual executable rendering responses at runtime.
`personal.file_content` mounts similar sub-paths, but they correspond to
files; its handlers just return the relevant blobs.

```
/bin/othismo.app/www/...           # whatever endpoints othismo.app handles are declared here
/bin/personal.file_content/...     # content cp'd into a read-only file tree, exposed here
```

## Operations

This implies the following operations:

* Modules exist under `/othismo/modules`.
* Instantiating a module requires a name and a path; the default instance
  name is `/...PATH/NAME.MODULE_NAME`.
* Instances are responsible for informing the namespace which messages they
  can handle (i.e. which sub-paths under them are valid).
    * Presumably, instances can register dynamically, so the namespace
      will not prematurely reject messages.
* `mount` — tell the namespace that a given instance handles some subset
  of the namespace.
* `sym_link` — tell the namespace to redirect messages sent to `A` to `B`.

## Building the example image

1. `import` `http` module
2. `import` `app` module
3. `import` `file_content`
4. `instantiate` `web` from `http` under `/net/http/`
    * creates `/net/http/web.http`
    * instance mounts `/net/http/web.http/sites`
5. `instantiate` `personal` from `file_content` under `/bin/`
    * file system contents are `mounted` under `/bin/personal.file_content/...`
6. `instantiate` `othismo` from `app` under `/bin/`
    * registers various endpoints under `/bin/othismo.app/www`
7. create sym links
