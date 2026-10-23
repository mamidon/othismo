import { Payload, Request, Response, Server, Status, Then, Verb, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { bson } from "othismo/bson"
import { Greeting } from "./Greeter.iface"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/**
 * Serves /greeting (write a Greeting to configure it). Greets every line on
 * the console, reports running statistics there, and has /shouter shout
 * each greeting.
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
    const msg = bson.decode(req.body)
    if (req.path != "/greeting") {
      req.fail(Status.NotFound)
    } else if (req.verb != Verb.Write || !(msg instanceof Greeting)) {
      req.fail(Status.NotSupported)
    } else {
      const g = msg as Greeting
      this.prefix = g.prefix
      this.excited = g.excited
      req.end()
    }
  }

  greet(name: string): void {
    this.lines++
    this.chars += name.length
    const greeting = Payload.text(this.prefix + name + (this.excited ? "!" : "."))
    const average = <f64>this.chars / <f64>this.lines
    ns.write("/dev/cons", greeting)
    ns.write("/dev/cons", Payload.text(this.lines.toString() + " lines, " + this.chars.toString() + " characters, " + average.toString() + " on average"))
    ns.write("/shouter", greeting)
  }
}

const greeter = new Greeter()

class OnName extends Then {
  answer(res: Response): void {
    const body = res.body
    if (body !== null) greeter.greet(body.asText())
  }
}

run(greeter)
