import { Request, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { ShouterServer, cons } from "./Shouter.oth"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Write a line of text to / and it appears on the console in capitals. */
class Shouter extends ShouterServer {
  write(req: Request, body: string): void {
    cons.write(body.toUpperCase())
    req.end()
  }
}

run(new Shouter())
