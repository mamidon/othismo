// Generated from TypesTests.oth by `othismo bindings`. Edit the header, not this file.

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
 * Tests the BSON format's field types in AssemblyScript: each test encodes
 * and decodes in the module itself, or round-trips a record through the
 * store, which the image wrote one to at startup.
 */
export class TypesTestsServer extends Server {
  /**
   * Runs a test, and answers `ok` if it passed, or an error, conventionally
   * `invalid`, whose description says why it didn't.
   *
   * `/tests/{name} write`
   */
  writeTests(req: Request, name: string): void { req.fail(Errno.NotSupported) }

  /**
   * The tests, a stat record each.
   *
   * `/tests list`
   */
  listTests(req: Request): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/tests/{name}", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { this.writeTests(req, m0![0]); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m1 = route("/tests", req.path)
    if (m1 !== null) {
      if (req.verb == Verb.List) { this.listTests(req); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}

/** The store to write records to and read them back from. */
export class StoreClient {
  constructor(public at: string) {}

  /** `/record write Record` */
  writeRecord(body: Record, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, "/record"), body.toPayload(), then)
  }

  /** `/record read -> Record` */
  readRecord(then: Then): Call {
    return ns.call(Verb.Read, under(this.at, "/record"), null, then)
  }
}

/** The store to write records to and read them back from. */
export const store = new StoreClient("/store")
