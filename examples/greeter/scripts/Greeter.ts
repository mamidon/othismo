import { Line } from "real-talk/env"
import { Request, Response, Server, Status, Then, Verb, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "real-talk/ns"
import { Greeting, Stats } from "../messages"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/**
 * Serves /greeting (write a Greeting to configure it). Greets every line on
 * the console, reports Stats there, and has /shouter shout each greeting.
 */
class Greeter extends Server {
  prefix: string = "Hi, "
  excited: bool = false
  lines: i32 = 0
  chars: i64 = 0

  start(): void {
    ns.watch("/dev/cons", new OnName())
  }

  serve(req: Request): void {
    const body = req.body
    if (req.path != "/greeting") {
      req.fail(Status.NotFound)
    } else if (req.verb != Verb.Write || !(body instanceof Greeting)) {
      req.fail(Status.NotSupported)
    } else {
      const g = body as Greeting
      this.prefix = g.prefix
      this.excited = g.excited
      req.end()
    }
  }

  greet(name: string): void {
    this.lines++
    this.chars += name.length
    const greeting = new Line(this.prefix + name + (this.excited ? "!" : "."))
    ns.write("/dev/cons", greeting)
    ns.write("/dev/cons", new Stats(this.lines, this.chars, <f64>this.chars / <f64>this.lines))
    ns.write("/shouter", greeting)
  }
}

const greeter = new Greeter()

class OnName extends Then {
  answer(res: Response): void {
    const body = res.body
    if (body instanceof Line) greeter.greet((body as Line).text)
  }
}

run(greeter)
