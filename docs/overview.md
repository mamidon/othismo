# Othismo

Othismo is a runtime for objects that can only talk to each other through
the interfaces they've declared.

With agentic development, so much code gets written that you can't
realistically review it in detail. Human attention has to go to what each
object does and how the objects talk to each other, not to exactly how each
one is implemented. But to trust code you haven't read, you have to trust its
boundaries: you need to know they're properly specified and that the object
can't step outside them. Traditional runtimes make it difficult to be certain of that.

## This site

This site is served by Othismo, from this image:

```text
import { HttpServer } from "../../sdk/objects/http/HttpServer.oth"
import { StaticSite } from "../../sdk/objects/site/StaticSite.oth"
import { Console, Listener, PackedFiles } from env

needs /net/tcp  Listener
needs /dev/cons Console

# Turns HTTP requests from the socket into requests for pathnames in its view
# and logs to the console
instance http : HttpServer {
  bind  /net/tcp  /net/tcp
  bind  /dev/cons /dev/cons
  mount /www pages
}

# Responds to messages with static content
instance pages : StaticSite {
  mount /files files readonly
}

# Said static content is found at image build time in the specified relative folder
instance files : PackedFiles { include "./public/" }

# Put http in the image's own view, so the requests below can configure it
mount /http http

# configure how the HTTP server routes hosts
write /http/hosts/* "/www"

# If another image imports this image, expose /hosts in the namespace this image serves
export /http/hosts at /hosts
```

[Take the tour](tour.md) to see how it works.
