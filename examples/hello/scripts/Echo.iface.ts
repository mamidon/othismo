// Generated from Echo.iface by `othismo bindings`. Edit the header, not this file.

import { Message, Reader, Writer, registerMessage } from "othismo"

/** A line, as Echo writes it back. */
export class Echoed extends Message {
  text: string

  constructor(text: string) {
    super()
    this.text = text
  }

  typeName(): string { return "Echoed" }
  encode(w: Writer): void { w.string("text", this.text) }
}
registerMessage("Echoed", (r: Reader): Message => new Echoed(r.string("text")))
