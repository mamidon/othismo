import { Message, OutPort, Receiver, bind, rt_inbox, rt_deliver } from "real-talk"
import { Line, ReadFile, FileContents, FileError } from "real-talk/env"
import { HttpRequest, HttpResponse } from "../messages"

export { rt_inbox, rt_deliver }

/** What we remember about a request while its file is being read. */
class Pending {
  constructor(public method: string, public path: string) {}
}

/**
 * Serves GET and HEAD by asking the files capability for the requested path.
 * It has no file system access of its own: it can only send ReadFile to
 * whatever the image wired `files` to.
 */
class StaticServer extends Receiver {
  pending: Map<u32, Pending> = new Map<u32, Pending>()

  constructor(
    private files: OutPort<ReadFile>,
    private responses: OutPort<HttpResponse>,
    private log: OutPort<Line>,
  ) {
    super()
  }

  receive(msg: Message): void {
    if (msg instanceof HttpRequest) {
      const req = msg as HttpRequest
      if (req.method != "GET" && req.method != "HEAD") {
        this.reply(req.conn, req.method, req.path, 405, "Method Not Allowed", "Allow: GET, HEAD\r\n", text("405 Method Not Allowed\n"))
        return
      }
      const path = req.path.endsWith("/") ? req.path + "index.html" : req.path
      this.pending.set(req.conn, new Pending(req.method, req.path))
      this.files.send(new ReadFile(req.conn, path))
    } else if (msg instanceof FileContents) {
      const f = msg as FileContents
      if (!this.pending.has(f.tag)) return
      const p = this.pending.get(f.tag)
      this.reply(f.tag, p.method, p.path, 200, "OK", "Content-Type: " + contentType(f.path) + "\r\n", f.data)
    } else if (msg instanceof FileError) {
      const e = msg as FileError
      if (!this.pending.has(e.tag)) return
      const p = this.pending.get(e.tag)
      if (e.reason == "not_found") {
        this.reply(e.tag, p.method, p.path, 404, "Not Found", "Content-Type: text/plain\r\n", text("404 Not Found\n"))
      } else if (e.reason == "directory" && !p.path.endsWith("/")) {
        const location = p.path + "/"
        this.reply(e.tag, p.method, p.path, 301, "Moved Permanently", "Location: " + encodeURI(location) + "\r\nContent-Type: text/plain\r\n", text("301 -> " + location + "\n"))
      } else if (e.reason == "directory") {
        this.reply(e.tag, p.method, p.path, 404, "Not Found", "Content-Type: text/plain\r\n", text("404 Not Found\n"))
      } else if (e.reason == "forbidden") {
        this.reply(e.tag, p.method, p.path, 403, "Forbidden", "Content-Type: text/plain\r\n", text("403 Forbidden\n"))
      } else {
        this.reply(e.tag, p.method, p.path, 500, "Internal Server Error", "Content-Type: text/plain\r\n", text("500 Internal Server Error\n"))
      }
    }
  }

  private reply(conn: u32, method: string, path: string, status: u16, reason: string, headers: string, body: ArrayBuffer): void {
    if (this.pending.has(conn)) this.pending.delete(conn)
    this.responses.send(new HttpResponse(conn, status, reason, headers, body, method == "HEAD"))
    this.log.send(new Line(method + " " + path + " " + status.toString()))
  }
}

function text(s: string): ArrayBuffer {
  return String.UTF8.encode(s)
}

function contentType(path: string): string {
  const dot = path.lastIndexOf(".")
  const ext = dot < 0 ? "" : path.substring(dot + 1).toLowerCase()
  if (ext == "html" || ext == "htm") return "text/html; charset=utf-8"
  if (ext == "css") return "text/css; charset=utf-8"
  if (ext == "js" || ext == "mjs") return "text/javascript; charset=utf-8"
  if (ext == "json") return "application/json"
  if (ext == "txt" || ext == "md") return "text/plain; charset=utf-8"
  if (ext == "svg") return "image/svg+xml"
  if (ext == "png") return "image/png"
  if (ext == "jpg" || ext == "jpeg") return "image/jpeg"
  if (ext == "gif") return "image/gif"
  if (ext == "ico") return "image/x-icon"
  if (ext == "wasm") return "application/wasm"
  return "application/octet-stream"
}

bind(new StaticServer(new OutPort<ReadFile>(0), new OutPort<HttpResponse>(1), new OutPort<Line>(2)))
