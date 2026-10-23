import { OnText, Request, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { Greeting, GreeterServer, cons, shouter } from "./Greeter.oth"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/**
 * Serves /greeting (write a Greeting to configure it). Greets every line on
 * the console, reports running statistics there, and has /shouter shout
 * each greeting.
 */
class Greeter extends GreeterServer {
  prefix: string = "Hi, "
  excited: bool = false
  lines: i32 = 0
  chars: i64 = 0

  start(): void {
    cons.watch(new OnName())
  }

  writeGreeting(req: Request, g: Greeting): void {
    this.prefix = g.prefix
    this.excited = g.excited
    req.end()
  }

  greet(name: string): void {
    this.lines++
    this.chars += name.length
    const greeting = this.prefix + name + (this.excited ? "!" : ".")
    const average = <f64>this.chars / <f64>this.lines
    cons.write(greeting)
    cons.write(this.lines.toString() + " lines, " + this.chars.toString() + " characters, " + average.toString() + " on average")
    shouter.write(greeting)
  }
}

const greeter = new Greeter()

class OnName extends OnText {
  text(line: string, done: bool): void {
    greeter.greet(line)
  }
}

run(greeter)
