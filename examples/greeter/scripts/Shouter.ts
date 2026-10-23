import { Payload, Request, Server, Errno, Verb, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Write a line of text to / and it appears on the console in capitals. */
class Shouter extends Server {
  serve(req: Request): void {
    const body = req.body
    if (req.path != "/") {
      req.fail(Errno.NotFound)
    } else if (req.verb != Verb.Write || body === null || !body.isText()) {
      req.fail(Errno.NotSupported)
    } else {
      ns.write("/dev/cons", Payload.text(body.asText().toUpperCase()))
      req.end()
    }
  }
}

run(new Shouter())
