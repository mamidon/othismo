import { OCTETS, OnText, Payload, Request, Response, Stat, Status, Then, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { HttpServerServer, cons, tcp } from "./HttpServer.oth"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Requests whose head exceeds this are refused with 431. */
const MAX_HEAD: i32 = 16 * 1024
/** Bodies above this are refused with 413. */
const MAX_BODY: i32 = 1024 * 1024

/**
 * Serves HTTP/1.1 on /net/tcp by turning each request into a request of the
 * namespace: GET is Read and HEAD is Stat, at the host's root (a path in our
 * view) plus the URL path. The answer becomes the response. Other methods are
 * refused, as its header only needs reads. It knows nothing about files.
 *
 * Its own tree is /hosts/<name>, which only the operator sets: write it a
 * path in our view to serve that host from there, or remove it to stop
 * serving it. Its base class, generated from its header, routes those
 * requests to the methods below. `*` is used for any host not listed. Every
 * response is logged to /dev/cons. One request per connection; every
 * response closes it.
 */
class HttpServer extends HttpServerServer {
  /** Host name (lowercase, no port) → root path in our view. */
  hosts: Map<string, string> = new Map<string, string>()

  start(): void {
    tcp.readAddress(new OnAddress())
    tcp.watchAccept(new OnAccept())
  }

  listHosts(req: Request): void {
    const names = this.hosts.keys()
    names.sort()
    for (let i = 0; i < names.length; i++) req.stream(Stat.file(names[i]).payload())
    req.end()
  }

  listHostsIndex(req: Request): void { this.listHosts(req) }

  writeHosts(req: Request, name: string, root: string): void {
    this.hosts.set(name.toLowerCase(), root)
    req.end()
  }

  removeHosts(req: Request, name: string): void {
    this.hosts.delete(name.toLowerCase())
    req.end()
  }

  readHosts(req: Request, name: string): void {
    if (!this.hosts.has(name.toLowerCase())) req.fail(Status.NotFound)
    else req.reply(Payload.text(this.hosts.get(name.toLowerCase())))
  }

  /** Where requests for `host` go, or null if it isn't served. */
  rootFor(host: string): string | null {
    if (this.hosts.has(host)) return this.hosts.get(host)
    if (this.hosts.has("*")) return this.hosts.get("*")
    return null
  }
}

const server = new HttpServer()

class OnAddress extends OnText {
  text(address: string, done: bool): void {
    cons.write("listening on http://" + address + "/")
  }
}

class OnAccept extends OnText {
  text(conn: string, done: bool): void {
    tcp.watchData(conn, new Connection(conn))
  }
}

/** One connection: buffers bytes until a whole request has arrived. */
class Connection extends Then {
  buf: ArrayBuffer = new ArrayBuffer(0)
  /** The request has been handed on; anything else received is ignored. */
  answering: bool = false

  constructor(public conn: string) {
    super()
  }

  answer(res: Response): void {
    const body = res.body
    if (this.answering || body === null) return // done, or the peer went away
    this.buf = concat(this.buf, body.bytes)
    this.parse()
  }

  private parse(): void {
    const buf = this.buf
    const end = findHeadEnd(Uint8Array.wrap(buf))
    if (end < 0) {
      if (buf.byteLength > MAX_HEAD) this.fail(431, "Request Header Fields Too Large")
      return
    }
    if (end > MAX_HEAD) {
      this.fail(431, "Request Header Fields Too Large")
      return
    }

    const lines = String.UTF8.decode(buf.slice(0, end)).split("\r\n")
    const start = lines[0].split(" ")
    if (start.length != 3 || !start[2].startsWith("HTTP/1.") || !start[1].startsWith("/")) {
      this.fail(400, "Bad Request")
      return
    }

    let host = ""
    let length: i32 = 0
    for (let i = 1; i < lines.length; i++) {
      const colon = lines[i].indexOf(":")
      if (colon <= 0) {
        this.fail(400, "Bad Request")
        return
      }
      const name = lines[i].substring(0, colon).trim().toLowerCase()
      const value = lines[i].substring(colon + 1).trim()
      if (name == "content-length") {
        if (!isDigits(value) || value.length > 9) {
          this.fail(400, "Bad Request")
          return
        }
        length = I32.parseInt(value)
      } else if (name == "host") {
        const port = value.lastIndexOf(":")
        host = (port > 0 && !value.endsWith("]") ? value.substring(0, port) : value).toLowerCase()
      }
    }
    if (length > MAX_BODY) {
      this.fail(413, "Content Too Large")
      return
    }

    const bodyStart = end + 4
    if (buf.byteLength < bodyStart + length) return // wait for the rest of the body

    const method = start[0]
    const target = start[1]
    const q = target.indexOf("?")
    const path = percentDecode(q < 0 ? target : target.substring(0, q))
    if (path == null) {
      this.fail(400, "Bad Request")
      return
    }
    this.answering = true
    this.buf = new ArrayBuffer(0)

    const respond = new Respond(this.conn, method, path)
    const root = server.rootFor(host)
    if (root == null) {
      respond.send(404, "Not Found", "Content-Type: text/plain\r\n", text("404 unknown host\n"))
      return
    }
    const full = ns.join(root, path)
    if (full == null) {
      respond.send(400, "Bad Request", "Content-Type: text/plain\r\n", text("400 Bad Request\n"))
      return
    }
    const plain = "Content-Type: text/plain\r\n"
    if (method == "GET") ns.read(full, respond)
    else if (method == "HEAD") ns.stat(full, respond)
    else if (method == "PUT" || method == "DELETE" || method == "POST") {
      respond.send(405, "Method Not Allowed", "Allow: GET, HEAD\r\n" + plain, text("405 Method Not Allowed\n"))
    } else respond.send(501, "Not Implemented", plain, text("501 Not Implemented\n"))
  }

  /** Answers a malformed request directly. */
  private fail(status: u16, reason: string): void {
    this.answering = true
    new Respond(this.conn, "-", "-").send(status, reason, "Content-Type: text/plain\r\n", text(status.toString() + " " + reason + "\n"))
  }
}

/** Turns the namespace's answer to one HTTP request into the response. */
class Respond extends Then {
  sent: bool = false

  constructor(public conn: string, public method: string, public path: string) {
    super()
  }

  answer(res: Response): void {
    if (this.sent) return
    const body = res.body
    const stat = Stat.decode(body)
    const plain = "Content-Type: text/plain\r\n"
    const directory = res.status.is(Status.IsDirectory) || (stat !== null && stat.dir)
    if (directory && !this.path.endsWith("/")) {
      const location = this.path + "/"
      this.send(301, "Moved Permanently", "Location: " + encodeURI(location) + "\r\n" + plain, text("301 -> " + location + "\n"))
    } else if (directory || res.status.is(Status.NotFound)) {
      this.send(404, "Not Found", plain, text("404 Not Found\n"))
    } else if (res.status.is(Status.ReadOnly) || res.status.is(Status.NotSupported)) {
      this.send(405, "Method Not Allowed", "Allow: GET, HEAD\r\n" + plain, text("405 Method Not Allowed\n"))
    } else if (res.status.is(Status.Forbidden)) {
      this.send(403, "Forbidden", plain, text("403 Forbidden\n"))
    } else if (res.status.is(Status.Invalid)) {
      this.send(400, "Bad Request", plain, text("400 Bad Request\n"))
    } else if (!res.ok) {
      this.send(500, "Internal Server Error", plain, text("500 Internal Server Error\n"))
    } else if (stat !== null) {
      // HEAD: the size without the bytes.
      this.send(200, "OK", "Content-Type: " + (stat.type.length > 0 ? stat.type : OCTETS) + "\r\n", new ArrayBuffer(0), <i64>stat.length)
    } else if (body !== null) {
      this.send(200, "OK", "Content-Type: " + (body.type.length > 0 ? body.type : OCTETS) + "\r\n", body.bytes)
    } else {
      this.send(204, "No Content", "", new ArrayBuffer(0))
    }
  }

  /**
   * Writes the response and closes the connection. A `headLength` of 0 or
   * more is a HEAD response: that Content-Length, and no body.
   */
  send(status: u16, reason: string, headers: string, body: ArrayBuffer, headLength: i64 = -1): void {
    this.sent = true
    const length = headLength >= 0 ? headLength : <i64>body.byteLength
    const bytes = String.UTF8.encode(
      "HTTP/1.1 " + status.toString() + " " + reason + "\r\n" +
      headers +
      "Content-Length: " + length.toString() + "\r\n" +
      "Connection: close\r\n\r\n",
    )
    tcp.writeData(this.conn, new Payload(OCTETS, headLength >= 0 ? bytes : concat(bytes, body)))
    tcp.remove(this.conn)
    cons.write(this.method + " " + this.path + " " + status.toString())
  }
}

function text(s: string): ArrayBuffer {
  return String.UTF8.encode(s)
}

function concat(a: ArrayBuffer, b: ArrayBuffer): ArrayBuffer {
  const out = new ArrayBuffer(a.byteLength + b.byteLength)
  memory.copy(changetype<usize>(out), changetype<usize>(a), <usize>a.byteLength)
  memory.copy(changetype<usize>(out) + <usize>a.byteLength, changetype<usize>(b), <usize>b.byteLength)
  return out
}

/** Index of the blank line ending the request head ("\r\n\r\n"), or -1. */
function findHeadEnd(b: Uint8Array): i32 {
  for (let i = 0; i + 3 < b.length; i++) {
    if (b[i] == 13 && b[i + 1] == 10 && b[i + 2] == 13 && b[i + 3] == 10) return i
  }
  return -1
}

function isDigits(s: string): bool {
  if (s.length == 0) return false
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i)
    if (c < 48 || c > 57) return false
  }
  return true
}

function hexValue(c: i32): i32 {
  if (c >= 48 && c <= 57) return c - 48
  if (c >= 65 && c <= 70) return c - 55
  if (c >= 97 && c <= 102) return c - 87
  return -1
}

/** Decodes %XX escapes as UTF-8; null if an escape or the result is invalid. */
function percentDecode(s: string): string | null {
  const raw = String.UTF8.encode(s)
  const src = Uint8Array.wrap(raw)
  const out = new Uint8Array(src.length)
  let n = 0
  for (let i = 0; i < src.length; i++) {
    let c = <i32>src[i]
    if (c == 37) { // '%'
      if (i + 2 >= src.length) return null
      const hi = hexValue(src[i + 1])
      const lo = hexValue(src[i + 2])
      if (hi < 0 || lo < 0) return null
      c = hi * 16 + lo
      if (c == 0) return null
      i += 2
    }
    out[n++] = <u8>c
  }
  return String.UTF8.decode(out.buffer.slice(0, n))
}

run(server)
