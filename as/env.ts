// Generated from env by `othismo bindings`. Edit the header, not this file.

import { Message, Reader, Writer, registerMessage } from "./bson"

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

/** How a test went. */
export class Outcome extends Message {
  passed: bool
  /** Why it failed, or anything worth saying about it. */
  detail: string

  constructor(passed: bool, detail: string) {
    super()
    this.passed = passed
    this.detail = detail
  }

  typeName(): string { return "Outcome" }
  encode(w: Writer): void {
    w.bool("passed", this.passed)
    w.string("detail", this.detail)
  }
}
registerMessage("Outcome", (r: Reader): Message => new Outcome(r.bool("passed"), r.string("detail")))
