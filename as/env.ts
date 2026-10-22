// Generated from env by `othismo bindings`. Edit the header, not this file.

import { Message, Reader, Writer, registerMessage } from "./index"

/** A string value: a setting, an address, a name. */
export class Text extends Message {
  text: string

  constructor(text: string) {
    super()
    this.text = text
  }

  typeName(): string { return "Text" }
  encode(w: Writer): void { w.string("text", this.text) }
}
registerMessage("Text", (r: Reader): Message => new Text(r.string("text")))

/** The contents of something, as `read` answers. */
export class Data extends Message {
  /** Its MIME type, or "" if it isn't known. */
  mime: string
  data: ArrayBuffer

  constructor(mime: string, data: ArrayBuffer) {
    super()
    this.mime = mime
    this.data = data
  }

  typeName(): string { return "Data" }
  encode(w: Writer): void {
    w.string("mime", this.mime)
    w.bytes("data", this.data)
  }
}
registerMessage("Data", (r: Reader): Message => new Data(r.string("mime"), r.bytes("data")))

/** What `stat` answers. */
export class Info extends Message {
  /** "file" or "dir". */
  kind: string
  /** In bytes; 0 for a directory. */
  size: i64
  /** Its MIME type, or "" if it isn't known. */
  mime: string

  constructor(kind: string, size: i64, mime: string) {
    super()
    this.kind = kind
    this.size = size
    this.mime = mime
  }

  typeName(): string { return "Info" }
  encode(w: Writer): void {
    w.string("kind", this.kind)
    w.i64("size", this.size)
    w.string("mime", this.mime)
  }
}
registerMessage("Info", (r: Reader): Message => new Info(r.string("kind"), r.i64("size"), r.string("mime")))

/** One name in a `list` stream, or what `create` made. */
export class Entry extends Message {
  name: string
  /** "file" or "dir". */
  kind: string

  constructor(name: string, kind: string) {
    super()
    this.name = name
    this.kind = kind
  }

  typeName(): string { return "Entry" }
  encode(w: Writer): void {
    w.string("name", this.name)
    w.string("kind", this.kind)
  }
}
registerMessage("Entry", (r: Reader): Message => new Entry(r.string("name"), r.string("kind")))

/** One line of text, without its line ending. */
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

/** Configures a Tcp listener. */
export class Listen extends Message {
  /** The address to listen on, e.g. "127.0.0.1:8080". */
  address: string

  constructor(address: string) {
    super()
    this.address = address
  }

  typeName(): string { return "Listen" }
  encode(w: Writer): void { w.string("address", this.address) }
}
registerMessage("Listen", (r: Reader): Message => new Listen(r.string("address")))

/** A new connection. */
export class Accepted extends Message {
  /** Its name under the listener: watch, write or remove `/<conn>`. */
  conn: string
  /** The remote address. */
  peer: string

  constructor(conn: string, peer: string) {
    super()
    this.conn = conn
    this.peer = peer
  }

  typeName(): string { return "Accepted" }
  encode(w: Writer): void {
    w.string("conn", this.conn)
    w.string("peer", this.peer)
  }
}
registerMessage("Accepted", (r: Reader): Message => new Accepted(r.string("conn"), r.string("peer")))

/** Configures Files. */
export class FilesRoot extends Message {
  /** The directory served, relative to the image. */
  path: string

  constructor(path: string) {
    super()
    this.path = path
  }

  typeName(): string { return "FilesRoot" }
  encode(w: Writer): void { w.string("path", this.path) }
}
registerMessage("FilesRoot", (r: Reader): Message => new FilesRoot(r.string("path")))
