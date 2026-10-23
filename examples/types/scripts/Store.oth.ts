// Generated from Store.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"
import { Payload } from "othismo"
import { Message, Reader, Writer, bson, registerMessage } from "othismo/bson"

/** A field of each type. */
export class Record extends Message {
  name: string
  count: i32

  constructor(name: string, count: i32) {
    super()
    this.name = name
    this.count = count
  }

  /** The `Record` in `p`, or null if it holds none. */
  static fromPayload(p: Payload | null): Record | null {
    const m = bson.decode(p)
    return m instanceof Record ? changetype<Record>(m) : null
  }
  toPayload(): Payload { return bson.payload(this) }

  typeName(): string { return "Record" }
  encode(w: Writer): void {
    w.string("name", this.name)
    w.i32("count", this.count)
  }
}
registerMessage("Record", (r: Reader): Message => new Record(r.string("name"), r.i32("count")))

/**
 * Keeps the last `Record` written to it, to read back: a payload of each of
 * the BSON format's field types, sent and received.
 */
export class StoreServer extends Server {
  /**
   * The last record written.
   *
   * `/record write Record`
   */
  writeRecord(req: Request, body: Record): void { req.fail(Errno.NotSupported) }

  /**
   * The last record written.
   *
   * `/record read -> Record`
   */
  readRecord(req: Request): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/record", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { const body = Record.fromPayload(req.body); if (body !== null) { this.writeRecord(req, body); return } }
      if (req.verb == Verb.Read) { this.readRecord(req); return }
      if (req.verb == Verb.Write) { req.fail(Errno.Invalid, "expected a Record"); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}
