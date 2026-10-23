// Generated from Greeter.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"

/**
 * Greets every name typed on the console, reports running statistics
 * there, and has `/shouter` shout each greeting.
 */
export class GreeterServer extends Server {
  /**
   * How to greet: a `Greeting`, a prefix and whether to be excited
   * about it.
   *
   * `/greeting write application/bson`
   */
  writeGreeting(req: Request, body: Payload): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/greeting", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { const body = payloadOf(req, "application/bson"); if (body !== null) this.writeGreeting(req, body!); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}

/** The console: the names, and where greetings and statistics go. */
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

/** The console: the names, and where greetings and statistics go. */
export const cons = new ConsClient("/dev/cons")

/** Something to shout each greeting. */
export class ShouterClient {
  constructor(public at: string) {}

  /** `/ write text/plain` */
  write(body: string, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, ""), Payload.text(body, "text/plain"), then)
  }
}

/** Something to shout each greeting. */
export const shouter = new ShouterClient("/shouter")
