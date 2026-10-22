import { Line } from "othismo/env"
import { Request, Server, Status, Verb, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo/ns"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Write a Line to / and it appears on the console in capitals. */
class Shouter extends Server {
  serve(req: Request): void {
    const body = req.body
    if (req.path != "/") {
      req.fail(Status.NotFound)
    } else if (req.verb != Verb.Write || !(body instanceof Line)) {
      req.fail(Status.NotSupported)
    } else {
      ns.write("/dev/cons", new Line((body as Line).text.toUpperCase()))
      req.end()
    }
  }
}

run(new Shouter())
