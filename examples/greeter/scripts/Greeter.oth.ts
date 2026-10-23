// Generated from Greeter.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"
import { Payload } from "othismo"
import { Message, Reader, Writer, bson, registerMessage } from "othismo/bson"

/** Configures a Greeter; sent once at startup by the image. */
export class Greeting extends Message {
  /** Put before each name. */
  prefix: string
  /** End with `!` rather than `.`. */
  excited: bool

  constructor(prefix: string, excited: bool) {
    super()
    this.prefix = prefix
    this.excited = excited
  }

  /** The `Greeting` in `p`, or null if it holds none. */
  static fromPayload(p: Payload | null): Greeting | null {
    const m = bson.decode(p)
    return m instanceof Greeting ? changetype<Greeting>(m) : null
  }
  toPayload(): Payload { return bson.payload(this) }

  typeName(): string { return "Greeting" }
  encode(w: Writer): void {
    w.string("prefix", this.prefix)
    w.bool("excited", this.excited)
  }
}
registerMessage("Greeting", (r: Reader): Message => new Greeting(r.string("prefix"), r.bool("excited")))

/**
 * Greets every name typed on the console, reports running statistics
 * there, and has `/shouter` shout each greeting.
 */
export class GreeterServer extends Server {
  /**
   * How to greet: a `Greeting`, a prefix and whether to be excited
   * about it.
   *
   * `/greeting write Greeting`
   */
  writeGreeting(req: Request, body: Greeting): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/greeting", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { const body = Greeting.fromPayload(req.body); if (body !== null) { this.writeGreeting(req, body); return } }
      if (req.verb == Verb.Write) { req.fail(Errno.Invalid, "expected a Greeting"); return }
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
