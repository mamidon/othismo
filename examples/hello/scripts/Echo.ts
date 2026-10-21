import { Line } from "real-talk/env"
import { Response, Server, Then, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "real-talk/ns"
import { Echoed } from "../messages"

// The host's entry points into this module.
export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Each line watched on the console goes back to it. */
class Echo extends Server {
  start(): void {
    ns.watch("/dev/cons", new OnLine())
  }
}

class OnLine extends Then {
  answer(res: Response): void {
    const body = res.body
    if (body instanceof Line) ns.write("/dev/cons", new Echoed((body as Line).text))
  }
}

run(new Echo())
