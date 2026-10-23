// Generated from env by `othismo bindings`. Edit the header, not this file.

import { Message, Reader, Writer, registerMessage } from "./bson"

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
