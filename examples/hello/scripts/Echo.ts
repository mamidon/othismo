import { Response, Server, Then, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"

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
    const line = res.body
    if (line !== null) ns.write("/dev/cons", line)
  }
}

run(new Echo())
