import { Message, Reader, Writer, registerMessage } from "real-talk"

/**
 * A parsed HTTP/1.x request. `path` is percent-decoded and has the query
 * string removed. `headers` holds one "name: value\r\n" line per header,
 * with names lowercased; use `header()` to look one up.
 */
export class HttpRequest extends Message {
  conn: u32
  method: string
  path: string
  query: string
  version: string
  headers: string
  body: ArrayBuffer

  constructor(conn: u32, method: string, path: string, query: string, version: string, headers: string, body: ArrayBuffer) {
    super()
    this.conn = conn
    this.method = method
    this.path = path
    this.query = query
    this.version = version
    this.headers = headers
    this.body = body
  }

  /** The value of header `name` (case-insensitive), or "" if absent. */
  header(name: string): string {
    const want = name.toLowerCase() + ": "
    const lines = this.headers.split("\r\n")
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].startsWith(want)) return lines[i].substring(want.length)
    }
    return ""
  }

  typeName(): string { return "HttpRequest" }
  encode(w: Writer): void {
    w.i64("conn", this.conn)
    w.string("method", this.method)
    w.string("path", this.path)
    w.string("query", this.query)
    w.string("version", this.version)
    w.string("headers", this.headers)
    w.bytes("body", this.body)
  }
}
registerMessage("HttpRequest", (r: Reader): Message => new HttpRequest(
  <u32>r.i64("conn"), r.string("method"), r.string("path"), r.string("query"),
  r.string("version"), r.string("headers"), r.bytes("body"),
))

/**
 * A response to the request that arrived on `conn`. `headers` holds extra
 * "Name: value\r\n" lines; the codec adds Content-Length and Connection.
 * With `head` set, the body is measured but not sent (for HEAD requests).
 */
export class HttpResponse extends Message {
  conn: u32
  status: u16
  reason: string
  headers: string
  body: ArrayBuffer
  head: bool

  constructor(conn: u32, status: u16, reason: string, headers: string, body: ArrayBuffer, head: bool) {
    super()
    this.conn = conn
    this.status = status
    this.reason = reason
    this.headers = headers
    this.body = body
    this.head = head
  }

  typeName(): string { return "HttpResponse" }
  encode(w: Writer): void {
    w.i64("conn", this.conn)
    w.i32("status", this.status)
    w.string("reason", this.reason)
    w.string("headers", this.headers)
    w.bytes("body", this.body)
    w.bool("head", this.head)
  }
}
registerMessage("HttpResponse", (r: Reader): Message => new HttpResponse(
  <u32>r.i64("conn"), <u16>r.i32("status"), r.string("reason"),
  r.string("headers"), r.bytes("body"), r.bool("head"),
))
