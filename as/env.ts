// Message types the runtime and its environment objects understand, and the
// conventional bodies for the namespace verbs.
//
// These are always in scope in an image. Object modules import them with
// `import { ... } from "real-talk/env"`. The runtime embeds this file, and an
// image that redeclares one of these types must give it exactly these fields.

import { Message, Reader, Writer, registerMessage } from "./index"

// ---------------------------------------------------------------------------
// Conventional bodies

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

/** The contents of something, as `Read` answers. `mime` may be "". */
export class Data extends Message {
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

/** What `Stat` answers. `kind` is "file" or "dir". */
export class Info extends Message {
  kind: string
  size: i64
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

/** One name in a `List` stream, or what `Create` made. `kind` is "file" or "dir". */
export class Entry extends Message {
  name: string
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

// ---------------------------------------------------------------------------
// Environment.Console
//
//   Watch /   a stream of Line, one per line of stdin, ending at EOF
//   Write /   prints the body

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

// ---------------------------------------------------------------------------
// Environment.Tcp: a TCP listener.
//
//   Read   /address  Text: the bound address, e.g. "127.0.0.1:8080"
//   Watch  /accept   a stream of Accepted
//   Watch  /<conn>   a stream of Data received, ending when the peer closes
//   Write  /<conn>   sends a Data's bytes
//   Remove /<conn>   closes our side once everything written has been sent
//
//   config (image only): send Listen { address: "..." }

/** Config: the address to listen on, e.g. "127.0.0.1:8080". */
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

/** A new connection: `conn` names it under the listener, `peer` is the remote address. */
export class Accepted extends Message {
  conn: string
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

// ---------------------------------------------------------------------------
// Environment.Files: read-only access to one directory tree.
//
//   Read   Data, with a MIME type from the extension; IsDirectory for a directory
//   Stat   Info
//   List   a stream of Entry, sorted by name
//
//   config (image only): send FilesRoot { path: "..." }

/** Config: the directory served, relative to the image file. */
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
