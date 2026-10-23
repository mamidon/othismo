// Generated from StaticSite.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"
import { FileTreeClient } from "othismo/env"

/** Serves a static site from a files tree it is given read-only. */
export class StaticSiteServer extends Server {
  /**
   * How many requests it has forwarded, as decimal text.
   *
   * `/hits read -> text/plain`
   */
  readHits(req: Request): void { req.fail(Errno.NotSupported) }

  /** `/** read -> *\/*` */
  readAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  /** `/** stat` */
  statAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  /** `/** list` */
  listAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/hits", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Read) { this.readHits(req); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m1 = route("/**", req.path)
    if (m1 !== null) {
      if (req.verb == Verb.Read) { this.readAny(req, m1![0]); return }
      if (req.verb == Verb.Stat) { this.statAny(req, m1![0]); return }
      if (req.verb == Verb.List) { this.listAny(req, m1![0]); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}

/** The site's files. Nothing is ever written here. */
export const files = new FileTreeClient("/files")
