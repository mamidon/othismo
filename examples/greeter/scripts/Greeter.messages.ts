// Generated from Greeter.messages by `othismo bindings`. Edit the schemas, not this file.

import { Message, Reader, Writer, registerMessage } from "othismo/bson"

/** Configures a Greeter; sent once at startup by the image. */
export class Greeting extends Message {
  /** Put before each name. */
  prefix: string
  /** End with `!` rather than `.`. */
  excited: bool

  constructor(prefix: string, excited: bool) {
    super()
    this.prefix = prefix
    this.excited = excited
  }

  typeName(): string { return "Greeting" }
  encode(w: Writer): void {
    w.string("prefix", this.prefix)
    w.bool("excited", this.excited)
  }
}
registerMessage("Greeting", (r: Reader): Message => new Greeting(r.string("prefix"), r.bool("excited")))
