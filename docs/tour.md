# A tour

The site is defined in two files. `site.oth` describes how the site itself is
put together: the HTTP server and what it serves. `static.oth` is a wrapper
that wires it up with the runtime configuration.

## static.oth

The imports bring in the site from `site.oth`, and `Listener` and
`Console`, whose implementations come from the environment itself: the TCP
connection and the console.

```text
import Site from "./site.oth"
import { Console, Listener } from env
```

`require listen` gives the default address the server listens on. It can be
overridden when the image is run, from the command line:
`--set listen=0.0.0.0:80`.

```text
require listen : text/plain = "127.0.0.1:8080"
```

The needs say where in the image's namespace the implementations of the TCP
listener and the console will be available.

```text
needs /net/tcp  Listener
needs /dev/cons Console
```

This declares the instance of the site and how its namespace is populated.

```text
instance web : Site {
  bind /net/tcp  /net/tcp
  bind /dev/cons /dev/cons
}
```

And finally, we tell the socket where to listen.

```text
write /net/tcp listen
```

## site.oth

This site is made of three objects: HttpServer, StaticSite and PackedFiles.
`site.oth` is the image that puts them together.

The imports bring in `HttpServer` and `StaticSite` from what passes for a
standard library, and `Console`, `Listener` and `PackedFiles` from the
runtime itself.

```text
import { HttpServer } from "../../sdk/objects/http/HttpServer.oth"
import { StaticSite } from "../../sdk/objects/site/StaticSite.oth"
import { Console, Listener, PackedFiles } from env
```

These are the same needs as in `static.oth`, but here they're filled by
whoever uses the site: `static.oth`, which passes on the real socket and
console it got from the environment.

```text
needs /net/tcp  Listener
needs /dev/cons Console
```

HttpServer is the only object with substantial logic inside it. It parses HTTP requests off
a socket and turns them into Othismo requests, addressed to paths in its
namespace. It routes each request by its host name, to the path that host is
configured with.

The `bind` lines give it the TCP socket and the console that this image was
given. `mount` puts a different object, one this image
defines, into its namespace: StaticSite, at `/www`.

```text
instance http : HttpServer {
  bind  /net/tcp  /net/tcp
  bind  /dev/cons /dev/cons
  mount /www pages
}
```

StaticSite receives requests from clients and, if they're appropriate,
answers them with the content of the files it has been given.

`readonly` means that requests to this path have to be read-only, as far as
the Othismo protocol is concerned.

```text
instance pages : StaticSite {
  mount /files files readonly
}
```

PackedFiles is the simplest. It's a set of files, baked into the image when
the image is built.

```text
instance files : PackedFiles { include "./public/" }
```

This puts HttpServer into the image's own namespace, so that the requests
after it can configure it.

```text
mount /http http
```

This configures how HttpServer routes hosts: every host goes to `/www`, where
StaticSite is, so a `GET` for `/foo` becomes a `read` of `/www/foo`.

```text
write /http/hosts/* "/www"
```

When another image imports this one and makes an instance of it, that
instance serves the hosts at `/hosts`, so the other image can configure the
server there.

```text
export /http/hosts at /hosts
```

> **Note:** some of this syntax is likely to change.
>
> - `bind` will probably go, in favour of mounting everything, whether it's an
>   instance of an object or a path mounted at a different location.
> - `readonly` will probably go too, once the Othismo protocol has real auth.
> - The syntax of `needs` and `export` could be better, compared with how
>   instances are mounted.
