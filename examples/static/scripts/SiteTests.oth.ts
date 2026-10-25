// Generated from SiteTests.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Payload, Request, Server, Status, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"
import { DialerClient } from "othismo/env"

/**
 * Tests the static site over HTTP, through its listener run in memory: each
 * test dials in with one request, and checks the response's status line.
 */
export class SiteTestsServer extends Server {
  /**
   * Runs a test, and answers `ok` if it passed, or an error, conventionally
   * `invalid`, whose description says why it didn't.
   *
   * `/tests/{name} write`
   */
  writeTests(req: Request, name: string): void { req.fail(Status.NotSupported) }

  /**
   * The tests, a stat record each.
   *
   * `/tests list`
   */
  listTests(req: Request): void { req.fail(Status.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/tests/{name}", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { this.writeTests(req, m0![0]); return }
      req.fail(Status.NotSupported)
      return
    }
    const m1 = route("/tests", req.path)
    if (m1 !== null) {
      if (req.verb == Verb.List) { this.listTests(req); return }
      req.fail(Status.NotSupported)
      return
    }
    req.fail(Status.NotFound)
  }
}

/** The site's listener, in memory, to dial. */
export const net = new DialerClient("/net")
