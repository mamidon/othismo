// Generated from env by `othismo bindings`. Edit the runtime's header, not this file.

import { Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under } from "./index"

/**
 * A console: lines in, and anything out. The runtime runs it on the host's
 * stdin and stdout.
 */
export class ConsoleClient {
  constructor(public at: string) {}

  /**
   * Watch for every line of input, until it ends; write to print.
   *
   * `/ watch -> text/plain*`
   */
  watch(then: Then): Call {
    return ns.call(Verb.Watch, under(this.at, ""), null, then)
  }

  /**
   * Watch for every line of input, until it ends; write to print.
   *
   * `/ write *\/*`
   */
  write(body: Payload, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, ""), body, then)
  }
}

/**
 * A console: lines in, and anything out. The runtime runs it on the host's
 * stdin and stdout.
 */
export class ConsoleServer extends Server {
  /**
   * Watch for every line of input, until it ends; write to print.
   *
   * `/ watch -> text/plain*`
   */
  watch(req: Request): void { req.fail(Errno.NotSupported) }

  /**
   * Watch for every line of input, until it ends; write to print.
   *
   * `/ write *\/*`
   */
  write(req: Request, body: Payload): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Watch) { this.watch(req); return }
      if (req.verb == Verb.Write) { const body = payloadOf(req, "*/*"); if (body !== null) this.write(req, body!); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}

/** The other side of a listener in memory: connect to it. */
export class DialerClient {
  constructor(public at: string) {}

  /**
   * Connect, sending the request's bytes, and get back everything the
   * server writes, until it closes the connection.
   *
   * `/dial create application/octet-stream -> application/octet-stream*`
   */
  createDial(body: Payload, then: Then | null = null): Call {
    return ns.call(Verb.Create, under(this.at, "/dial"), body, then)
  }
}

/** The other side of a listener in memory: connect to it. */
export class DialerServer extends Server {
  /**
   * Connect, sending the request's bytes, and get back everything the
   * server writes, until it closes the connection.
   *
   * `/dial create application/octet-stream -> application/octet-stream*`
   */
  createDial(req: Request, body: Payload): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/dial", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Create) { const body = payloadOf(req, "application/octet-stream"); if (body !== null) this.createDial(req, body!); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}

/** A tree of files, read-only. */
export class FileTreeClient {
  constructor(public at: string) {}

  /** `/** read -> *\/*` */
  readAny(rest: string, then: Then): Call {
    return ns.call(Verb.Read, under(this.at, rest), null, then)
  }

  /** `/** stat` */
  statAny(rest: string, then: Then): Call {
    return ns.call(Verb.Stat, under(this.at, rest), null, then)
  }

  /** `/** list` */
  listAny(rest: string, then: Then): Call {
    return ns.call(Verb.List, under(this.at, rest), null, then)
  }
}

/** A tree of files, read-only. */
export class FileTreeServer extends Server {
  /** `/** read -> *\/*` */
  readAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  /** `/** stat` */
  statAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  /** `/** list` */
  listAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/**", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Read) { this.readAny(req, m0![0]); return }
      if (req.verb == Verb.Stat) { this.statAny(req, m0![0]); return }
      if (req.verb == Verb.List) { this.listAny(req, m0![0]); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}

/** Read-only access to one directory on the host, and nothing above it. */
export class FilesClient {
  constructor(public at: string) {}

  /**
   * Configured by the image only, with the directory to serve, relative
   * to the image.
   *
   * `/ read -> *\/*`
   */
  read(then: Then): Call {
    return ns.call(Verb.Read, under(this.at, ""), null, then)
  }

  /**
   * Configured by the image only, with the directory to serve, relative
   * to the image.
   *
   * `/ stat`
   */
  stat(then: Then): Call {
    return ns.call(Verb.Stat, under(this.at, ""), null, then)
  }

  /**
   * Configured by the image only, with the directory to serve, relative
   * to the image.
   *
   * `/ list`
   */
  list(then: Then): Call {
    return ns.call(Verb.List, under(this.at, ""), null, then)
  }

  /** `/** read -> *\/*` */
  readAny(rest: string, then: Then): Call {
    return ns.call(Verb.Read, under(this.at, rest), null, then)
  }

  /** `/** stat` */
  statAny(rest: string, then: Then): Call {
    return ns.call(Verb.Stat, under(this.at, rest), null, then)
  }

  /** `/** list` */
  listAny(rest: string, then: Then): Call {
    return ns.call(Verb.List, under(this.at, rest), null, then)
  }
}

/** Read-only access to one directory on the host, and nothing above it. */
export class FilesServer extends Server {
  /**
   * Configured by the image only, with the directory to serve, relative
   * to the image.
   *
   * `/ operator write text/plain`
   */
  write(req: Request, body: string): void { req.fail(Errno.NotSupported) }

  /**
   * Configured by the image only, with the directory to serve, relative
   * to the image.
   *
   * `/ read -> *\/*`
   */
  read(req: Request): void { req.fail(Errno.NotSupported) }

  /**
   * Configured by the image only, with the directory to serve, relative
   * to the image.
   *
   * `/ stat`
   */
  stat(req: Request): void { req.fail(Errno.NotSupported) }

  /**
   * Configured by the image only, with the directory to serve, relative
   * to the image.
   *
   * `/ list`
   */
  list(req: Request): void { req.fail(Errno.NotSupported) }

  /** `/** read -> *\/*` */
  readAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  /** `/** stat` */
  statAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  /** `/** list` */
  listAny(req: Request, rest: string): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { const body = textOf(req, "text/plain"); if (body !== null) this.write(req, body!); return }
      if (req.verb == Verb.Read) { this.read(req); return }
      if (req.verb == Verb.Stat) { this.stat(req); return }
      if (req.verb == Verb.List) { this.list(req); return }
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

/**
 * A listener, and the connections it accepts. The runtime runs it as a TCP
 * listener on the address the image configures it with.
 */
export class ListenerClient {
  constructor(public at: string) {}

  /**
   * The bound address, e.g. "127.0.0.1:8080".
   *
   * `/address read -> text/plain`
   */
  readAddress(then: Then): Call {
    return ns.call(Verb.Read, under(this.at, "/address"), null, then)
  }

  /**
   * The name of every connection accepted, as it is.
   *
   * `/accept watch -> text/plain*`
   */
  watchAccept(then: Then): Call {
    return ns.call(Verb.Watch, under(this.at, "/accept"), null, then)
  }

  /**
   * A connection's bytes: watch for those received, until the peer
   * closes; write to send.
   *
   * `/{conn}/data watch -> application/octet-stream*`
   */
  watchData(conn: string, then: Then): Call {
    return ns.call(Verb.Watch, under(this.at, "/" + conn + "/data"), null, then)
  }

  /**
   * A connection's bytes: watch for those received, until the peer
   * closes; write to send.
   *
   * `/{conn}/data write application/octet-stream`
   */
  writeData(conn: string, body: Payload, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, "/" + conn + "/data"), body, then)
  }

  /**
   * The peer's address.
   *
   * `/{conn}/remote read -> text/plain`
   */
  readRemote(conn: string, then: Then): Call {
    return ns.call(Verb.Read, under(this.at, "/" + conn + "/remote"), null, then)
  }

  /**
   * Close our side, once everything written is sent.
   *
   * `/{conn} remove`
   */
  remove(conn: string, then: Then | null = null): Call {
    return ns.call(Verb.Remove, under(this.at, "/" + conn), null, then)
  }
}

/**
 * A listener, and the connections it accepts. The runtime runs it as a TCP
 * listener on the address the image configures it with.
 */
export class ListenerServer extends Server {
  /**
   * Configured by the image only, with the address to listen on, e.g.
   * "127.0.0.1:8080".
   *
   * `/ operator write text/plain`
   */
  write(req: Request, body: string): void { req.fail(Errno.NotSupported) }

  /**
   * The bound address, e.g. "127.0.0.1:8080".
   *
   * `/address read -> text/plain`
   */
  readAddress(req: Request): void { req.fail(Errno.NotSupported) }

  /**
   * The name of every connection accepted, as it is.
   *
   * `/accept watch -> text/plain*`
   */
  watchAccept(req: Request): void { req.fail(Errno.NotSupported) }

  /**
   * A connection's bytes: watch for those received, until the peer
   * closes; write to send.
   *
   * `/{conn}/data watch -> application/octet-stream*`
   */
  watchData(req: Request, conn: string): void { req.fail(Errno.NotSupported) }

  /**
   * A connection's bytes: watch for those received, until the peer
   * closes; write to send.
   *
   * `/{conn}/data write application/octet-stream`
   */
  writeData(req: Request, conn: string, body: Payload): void { req.fail(Errno.NotSupported) }

  /**
   * The peer's address.
   *
   * `/{conn}/remote read -> text/plain`
   */
  readRemote(req: Request, conn: string): void { req.fail(Errno.NotSupported) }

  /**
   * Close our side, once everything written is sent.
   *
   * `/{conn} remove`
   */
  remove(req: Request, conn: string): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { const body = textOf(req, "text/plain"); if (body !== null) this.write(req, body!); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m1 = route("/address", req.path)
    if (m1 !== null) {
      if (req.verb == Verb.Read) { this.readAddress(req); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m2 = route("/accept", req.path)
    if (m2 !== null) {
      if (req.verb == Verb.Watch) { this.watchAccept(req); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m3 = route("/{conn}/data", req.path)
    if (m3 !== null) {
      if (req.verb == Verb.Watch) { this.watchData(req, m3![0]); return }
      if (req.verb == Verb.Write) { const body = payloadOf(req, "application/octet-stream"); if (body !== null) this.writeData(req, m3![0], body!); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m4 = route("/{conn}/remote", req.path)
    if (m4 !== null) {
      if (req.verb == Verb.Read) { this.readRemote(req, m4![0]); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m5 = route("/{conn}", req.path)
    if (m5 !== null) {
      if (req.verb == Verb.Remove) { this.remove(req, m5![0]); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}

/**
 * The other side of a console in memory: type lines into it, and watch
 * what it prints.
 */
export class TerminalClient {
  constructor(public at: string) {}

  /**
   * Type a line, as if on stdin.
   *
   * `/typed write text/plain`
   */
  writeTyped(body: string, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, "/typed"), Payload.text(body, "text/plain"), then)
  }

  /**
   * Everything written to the console, as it is written.
   *
   * `/printed watch -> *\/**`
   */
  watchPrinted(then: Then): Call {
    return ns.call(Verb.Watch, under(this.at, "/printed"), null, then)
  }
}

/**
 * The other side of a console in memory: type lines into it, and watch
 * what it prints.
 */
export class TerminalServer extends Server {
  /**
   * Type a line, as if on stdin.
   *
   * `/typed write text/plain`
   */
  writeTyped(req: Request, body: string): void { req.fail(Errno.NotSupported) }

  /**
   * Everything written to the console, as it is written.
   *
   * `/printed watch -> *\/**`
   */
  watchPrinted(req: Request): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/typed", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { const body = textOf(req, "text/plain"); if (body !== null) this.writeTyped(req, body!); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m1 = route("/printed", req.path)
    if (m1 !== null) {
      if (req.verb == Verb.Watch) { this.watchPrinted(req); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}

/**
 * Something with tests to run, which `othismo test` finds by this trait and
 * runs, each in a fresh run of the image.
 */
export class TestSuiteClient {
  constructor(public at: string) {}

  /**
   * Runs a test, and answers `ok` if it passed, or an error, conventionally
   * `invalid`, whose description says why it didn't.
   *
   * `/tests/{name} write`
   */
  writeTests(name: string, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, "/tests/" + name), null, then)
  }

  /**
   * The tests, a stat record each.
   *
   * `/tests list`
   */
  listTests(then: Then): Call {
    return ns.call(Verb.List, under(this.at, "/tests"), null, then)
  }
}

/**
 * Something with tests to run, which `othismo test` finds by this trait and
 * runs, each in a fresh run of the image.
 */
export class TestSuiteServer extends Server {
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
