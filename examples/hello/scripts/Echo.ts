import { OnText, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { EchoServer, cons } from "./Echo.oth"

// The host's entry points into this module.
export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Each line watched on the console goes back to it. */
class Echo extends EchoServer {
  start(): void {
    cons.watch(new OnLine())
  }
}

class OnLine extends OnText {
  text(line: string, done: bool): void {
    cons.write(line)
  }
}

run(new Echo())
