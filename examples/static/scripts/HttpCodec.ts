import { Message, OutPort, Receiver, bind, rt_inbox, rt_deliver } from "real-talk"
import { SocketData, SocketClosed, SocketWrite, SocketClose } from "real-talk/env"
import { HttpRequest, HttpResponse } from "../messages"

export { rt_inbox, rt_deliver }

/** Requests whose head exceeds this are refused with 431. */
const MAX_HEAD: i32 = 16 * 1024
/** Bodies above this are refused with 413. */
const MAX_BODY: i32 = 1024 * 1024

/**
 * Speaks HTTP/1.1 over Environment.Socket: buffers bytes per connection until
 * a whole request has arrived, emits it as a typed HttpRequest, and turns the
 * HttpResponse that comes back into bytes. One request per connection; every
 * response closes it.
 */
class HttpCodec extends Receiver {
  /** Bytes received so far on connections still reading a request. */
  pending: Map<u32, ArrayBuffer> = new Map<u32, ArrayBuffer>()
  /** Connections whose request was handed on and await a response. */
  answering: Set<u32> = new Set<u32>()

  constructor(private socket: OutPort<Message>, private requests: OutPort<HttpRequest>) {
    super()
  }

  receive(msg: Message): void {
    if (msg instanceof SocketData) {
      const d = msg as SocketData
      if (this.answering.has(d.conn)) return // ignore anything after the request
      const buf = this.pending.has(d.conn) ? concat(this.pending.get(d.conn), d.data) : d.data
      this.pending.set(d.conn, buf)
      this.parse(d.conn, buf)
    } else if (msg instanceof SocketClosed) {
      const c = (msg as SocketClosed).conn
      if (this.pending.has(c)) this.pending.delete(c)
      if (this.answering.has(c)) this.answering.delete(c)
    } else if (msg instanceof HttpResponse) {
      this.respond(msg as HttpResponse)
    }
  }

  private parse(conn: u32, buf: ArrayBuffer): void {
    const bytes = Uint8Array.wrap(buf)
    const end = findHeadEnd(bytes)
    if (end < 0) {
      if (buf.byteLength > MAX_HEAD) this.fail(conn, 431, "Request Header Fields Too Large")
      return
    }
    if (end > MAX_HEAD) {
      this.fail(conn, 431, "Request Header Fields Too Large")
      return
    }

    const lines = String.UTF8.decode(buf.slice(0, end)).split("\r\n")
    const start = lines[0].split(" ")
    if (start.length != 3 || !start[2].startsWith("HTTP/1.") || !start[1].startsWith("/")) {
      this.fail(conn, 400, "Bad Request")
      return
    }

    let headers = ""
    let length: i32 = 0
    for (let i = 1; i < lines.length; i++) {
      const colon = lines[i].indexOf(":")
      if (colon <= 0) {
        this.fail(conn, 400, "Bad Request")
        return
      }
      const name = lines[i].substring(0, colon).trim().toLowerCase()
      const value = lines[i].substring(colon + 1).trim()
      if (name == "content-length") {
        if (!isDigits(value) || value.length > 9) {
          this.fail(conn, 400, "Bad Request")
          return
        }
        length = I32.parseInt(value)
      }
      headers += name + ": " + value + "\r\n"
    }
    if (length > MAX_BODY) {
      this.fail(conn, 413, "Content Too Large")
      return
    }

    const bodyStart = end + 4
    if (buf.byteLength < bodyStart + length) return // wait for the rest of the body

    const target = start[1]
    const q = target.indexOf("?")
    const rawPath = q < 0 ? target : target.substring(0, q)
    const query = q < 0 ? "" : target.substring(q + 1)
    const path = percentDecode(rawPath)
    if (path == null) {
      this.fail(conn, 400, "Bad Request")
      return
    }

    this.pending.delete(conn)
    this.answering.add(conn)
    const body = buf.slice(bodyStart, bodyStart + length)
    this.requests.send(new HttpRequest(conn, start[0], path, query, start[2], headers, body))
  }

  private respond(r: HttpResponse): void {
    if (!this.answering.has(r.conn)) return // the client went away
    this.answering.delete(r.conn)
    const head = "HTTP/1.1 " + r.status.toString() + " " + r.reason + "\r\n" +
      r.headers +
      "Content-Length: " + r.body.byteLength.toString() + "\r\n" +
      "Connection: close\r\n\r\n"
    const headBytes = String.UTF8.encode(head)
    this.socket.send(new SocketWrite(r.conn, r.head ? headBytes : concat(headBytes, r.body)))
    this.socket.send(new SocketClose(r.conn))
  }

  /** Answers a malformed request directly, without bothering the server. */
  private fail(conn: u32, status: u16, reason: string): void {
    this.pending.delete(conn)
    this.answering.add(conn)
    const body = String.UTF8.encode(status.toString() + " " + reason + "\n")
    this.respond(new HttpResponse(conn, status, reason, "Content-Type: text/plain\r\n", body, false))
  }
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

bind(new HttpCodec(new OutPort<Message>(0), new OutPort<HttpRequest>(1)))
