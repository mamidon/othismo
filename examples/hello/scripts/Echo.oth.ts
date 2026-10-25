// Generated from Echo.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Payload, Request, Server, Status, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"

/** Echoes every line typed on the console back to it. */
export class EchoServer extends Server {
  serve(req: Request): void {
    req.fail(Status.NotFound)
  }
}

/** The console: the lines to echo, and where they go. */
export class ConsClient {
  constructor(public at: string) {}

  /** `/ watch -> text/plain*` */
  watch(then: Then): Call {
    return ns.call(Verb.Watch, under(this.at, ""), null, then)
  }

  /** `/ write text/plain` */
  write(body: string, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, ""), Payload.text(body, "text/plain"), then)
  }
}

/** The console: the lines to echo, and where they go. */
export const cons = new ConsClient("/dev/cons")
