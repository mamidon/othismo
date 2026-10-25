// Generated from Shouter.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Payload, Request, Server, Status, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"

/** Writes each line it is given to the console, in capitals. */
export class ShouterServer extends Server {
  /** `/ write text/plain` */
  write(req: Request, body: string): void { req.fail(Status.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { const body = textOf(req, "text/plain"); if (body !== null) this.write(req, body!); return }
      req.fail(Status.NotSupported)
      return
    }
    req.fail(Status.NotFound)
  }
}

export class ConsClient {
  constructor(public at: string) {}

  /** `/ write text/plain` */
  write(body: string, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, ""), Payload.text(body, "text/plain"), then)
  }
}

export const cons = new ConsClient("/dev/cons")
