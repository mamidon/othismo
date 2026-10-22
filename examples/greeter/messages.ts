import { Message, Reader, Writer, registerMessage } from "othismo"

/** Configures a Greeter; sent once at startup by the image. */
export class Greeting extends Message {
  prefix: string
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

/** Running totals a Greeter reports after every line. */
export class Stats extends Message {
  lines: i32
  chars: i64
  average: f64

  constructor(lines: i32, chars: i64, average: f64) {
    super()
    this.lines = lines
    this.chars = chars
    this.average = average
  }

  typeName(): string { return "Stats" }
  encode(w: Writer): void {
    w.i32("lines", this.lines)
    w.i64("chars", this.chars)
    w.f64("average", this.average)
  }
}
registerMessage("Stats", (r: Reader): Message => new Stats(r.i32("lines"), r.i64("chars"), r.f64("average")))
