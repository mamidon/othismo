import { Message, Reader, Writer, registerMessage } from "real-talk"

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
