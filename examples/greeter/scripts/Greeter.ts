import { Message, OutPort, Receiver, bind, rt_inbox, rt_deliver } from "real-talk"
import { Line } from "real-talk/env"
import { Greeting, Stats } from "../messages"

export { rt_inbox, rt_deliver }

class Greeter extends Receiver {
  prefix: string = "Hi, "
  excited: bool = false
  lines: i32 = 0
  chars: i64 = 0

  constructor(private greeted: OutPort<Line>, private stats: OutPort<Stats>) {
    super()
  }

  receive(msg: Message): void {
    if (msg instanceof Greeting) {
      const g = msg as Greeting
      this.prefix = g.prefix
      this.excited = g.excited
    } else if (msg instanceof Line) {
      const name = (msg as Line).text
      this.lines++
      this.chars += name.length
      this.greeted.send(new Line(this.prefix + name + (this.excited ? "!" : ".")))
      this.stats.send(new Stats(this.lines, this.chars, <f64>this.chars / <f64>this.lines))
    }
  }
}

bind(new Greeter(new OutPort<Line>(0), new OutPort<Stats>(1)))
