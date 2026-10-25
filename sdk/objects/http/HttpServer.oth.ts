// Generated from HttpServer.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Payload, Request, Server, Status, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"
import { ListenerClient } from "othismo/env"

/**
 * Serves HTTP/1.1 on `/net/tcp`, turning each request into a request of its
 * view.
 */
export class HttpServerServer extends Server {
  /** `/hosts/ list` */
  listHostsIndex(req: Request): void { req.fail(Status.NotSupported) }

  /**
   * Write a path in its view to serve the host from there, or remove it
   * to stop serving it. `*` stands for any host not listed.
   *
   * `/hosts/{name} operator write text/plain`
   */
  writeHosts(req: Request, name: string, body: string): void { req.fail(Status.NotSupported) }

  /**
   * Write a path in its view to serve the host from there, or remove it
   * to stop serving it. `*` stands for any host not listed.
   *
   * `/hosts/{name} operator remove`
   */
  removeHosts(req: Request, name: string): void { req.fail(Status.NotSupported) }

  /**
   * Write a path in its view to serve the host from there, or remove it
   * to stop serving it. `*` stands for any host not listed.
   *
   * `/hosts/{name} read -> text/plain`
   */
  readHosts(req: Request, name: string): void { req.fail(Status.NotSupported) }

  /**
   * The hosts it serves.
   *
   * `/hosts list`
   */
  listHosts(req: Request): void { req.fail(Status.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/hosts/", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.List) { this.listHostsIndex(req); return }
      req.fail(Status.NotSupported)
      return
    }
    const m1 = route("/hosts/{name}", req.path)
    if (m1 !== null) {
      if (req.verb == Verb.Write) { const body = textOf(req, "text/plain"); if (body !== null) this.writeHosts(req, m1![0], body!); return }
      if (req.verb == Verb.Remove) { this.removeHosts(req, m1![0]); return }
      if (req.verb == Verb.Read) { this.readHosts(req, m1![0]); return }
      req.fail(Status.NotSupported)
      return
    }
    const m2 = route("/hosts", req.path)
    if (m2 !== null) {
      if (req.verb == Verb.List) { this.listHosts(req); return }
      req.fail(Status.NotSupported)
      return
    }
    req.fail(Status.NotFound)
  }
}

/** The listener it serves HTTP on. */
export const tcp = new ListenerClient("/net/tcp")

/** The access log. */
export class ConsClient {
  constructor(public at: string) {}

  /** `/ write text/plain` */
  write(body: string, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, ""), Payload.text(body, "text/plain"), then)
  }
}

/** The access log. */
export const cons = new ConsClient("/dev/cons")

/** The sites, at whichever mount points the hosts are configured with. */
export class AnyClient {
  constructor(public at: string) {}

  /** `/** read -> *\/*` */
  readAny(rest: string, then: Then): Call {
    return ns.call(Verb.Read, under(this.at, rest), null, then)
  }

  /** `/** stat` */
  statAny(rest: string, then: Then): Call {
    return ns.call(Verb.Stat, under(this.at, rest), null, then)
  }
}
