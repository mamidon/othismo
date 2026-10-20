// Messages understood by the runtime's environmental objects.
//
// Images import these with `import { ... } from "real-talk/env"`, and object
// modules do the same in AssemblyScript. The runtime embeds this file and
// checks that every environment type an image uses has exactly these fields.

import { Message, Reader, Writer, registerMessage } from "./index"

// ---------------------------------------------------------------------------
// Environment.Stdin / Environment.Stdout

/** One line of text, without its line ending. Emitted by Environment.Stdin. */
export class Line extends Message {
  text: string

  constructor(text: string) {
    super()
    this.text = text
  }

  typeName(): string { return "Line" }
  encode(w: Writer): void { w.string("text", this.text) }
}
registerMessage("Line", (r: Reader): Message => new Line(r.string("text")))

// ---------------------------------------------------------------------------
// Environment.Socket: a TCP listener.
//
//   in:  SocketWrite, SocketClose
//   out out:    SocketData, SocketClosed
//   out events: Listening
//   startup (exactly once, image only): Listen

/** Startup config: the address to listen on, e.g. "127.0.0.1:8080". */
export class Listen extends Message {
  address: string

  constructor(address: string) {
    super()
    this.address = address
  }

  typeName(): string { return "Listen" }
  encode(w: Writer): void { w.string("address", this.address) }
}
registerMessage("Listen", (r: Reader): Message => new Listen(r.string("address")))

/** Emitted once the socket is bound; `address` has the real port. */
export class Listening extends Message {
  address: string

  constructor(address: string) {
    super()
    this.address = address
  }

  typeName(): string { return "Listening" }
  encode(w: Writer): void { w.string("address", this.address) }
}
registerMessage("Listening", (r: Reader): Message => new Listening(r.string("address")))

/** Bytes received on connection `conn`. */
export class SocketData extends Message {
  conn: u32
  data: ArrayBuffer

  constructor(conn: u32, data: ArrayBuffer) {
    super()
    this.conn = conn
    this.data = data
  }

  typeName(): string { return "SocketData" }
  encode(w: Writer): void {
    w.i64("conn", this.conn)
    w.bytes("data", this.data)
  }
}
registerMessage("SocketData", (r: Reader): Message => new SocketData(<u32>r.i64("conn"), r.bytes("data")))

/** The peer closed connection `conn`, or it failed. */
export class SocketClosed extends Message {
  conn: u32

  constructor(conn: u32) {
    super()
    this.conn = conn
  }

  typeName(): string { return "SocketClosed" }
  encode(w: Writer): void { w.i64("conn", this.conn) }
}
registerMessage("SocketClosed", (r: Reader): Message => new SocketClosed(<u32>r.i64("conn")))

/** Writes bytes to connection `conn`. */
export class SocketWrite extends Message {
  conn: u32
  data: ArrayBuffer

  constructor(conn: u32, data: ArrayBuffer) {
    super()
    this.conn = conn
    this.data = data
  }

  typeName(): string { return "SocketWrite" }
  encode(w: Writer): void {
    w.i64("conn", this.conn)
    w.bytes("data", this.data)
  }
}
registerMessage("SocketWrite", (r: Reader): Message => new SocketWrite(<u32>r.i64("conn"), r.bytes("data")))

/** Closes connection `conn` after everything written so far. */
export class SocketClose extends Message {
  conn: u32

  constructor(conn: u32) {
    super()
    this.conn = conn
  }

  typeName(): string { return "SocketClose" }
  encode(w: Writer): void { w.i64("conn", this.conn) }
}
registerMessage("SocketClose", (r: Reader): Message => new SocketClose(<u32>r.i64("conn")))

// ---------------------------------------------------------------------------
// Environment.Files: read-only access to one directory tree.
//
//   in:  ReadFile
//   out out: FileContents, FileError
//   startup (exactly once, image only): FilesRoot

/** Startup config: the directory served, relative to the image file. */
export class FilesRoot extends Message {
  path: string

  constructor(path: string) {
    super()
    this.path = path
  }

  typeName(): string { return "FilesRoot" }
  encode(w: Writer): void { w.string("path", this.path) }
}
registerMessage("FilesRoot", (r: Reader): Message => new FilesRoot(r.string("path")))

/** Reads `path` (relative to the root; a leading "/" is ignored). `tag` is echoed back. */
export class ReadFile extends Message {
  tag: u32
  path: string

  constructor(tag: u32, path: string) {
    super()
    this.tag = tag
    this.path = path
  }

  typeName(): string { return "ReadFile" }
  encode(w: Writer): void {
    w.i64("tag", this.tag)
    w.string("path", this.path)
  }
}
registerMessage("ReadFile", (r: Reader): Message => new ReadFile(<u32>r.i64("tag"), r.string("path")))

/** The whole contents of the file a ReadFile asked for. */
export class FileContents extends Message {
  tag: u32
  path: string
  data: ArrayBuffer

  constructor(tag: u32, path: string, data: ArrayBuffer) {
    super()
    this.tag = tag
    this.path = path
    this.data = data
  }

  typeName(): string { return "FileContents" }
  encode(w: Writer): void {
    w.i64("tag", this.tag)
    w.string("path", this.path)
    w.bytes("data", this.data)
  }
}
registerMessage("FileContents", (r: Reader): Message => new FileContents(<u32>r.i64("tag"), r.string("path"), r.bytes("data")))

/**
 * A ReadFile failed. `reason` is "not_found", "directory" (the path is a
 * directory), "forbidden" (outside the root) or "io" (anything else).
 */
export class FileError extends Message {
  tag: u32
  path: string
  reason: string

  constructor(tag: u32, path: string, reason: string) {
    super()
    this.tag = tag
    this.path = path
    this.reason = reason
  }

  typeName(): string { return "FileError" }
  encode(w: Writer): void {
    w.i64("tag", this.tag)
    w.string("path", this.path)
    w.string("reason", this.reason)
  }
}
registerMessage("FileError", (r: Reader): Message => new FileError(<u32>r.i64("tag"), r.string("path"), r.string("reason")))
