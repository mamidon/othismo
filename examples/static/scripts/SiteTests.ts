import { Status, OCTETS, Payload, Request, Response, Stat, Then, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { SiteTestsServer, net } from "./SiteTests.oth"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** One request to the site, and how its status line should start. */
class Case {
  constructor(public name: string, public host: string, public line: string, public want: string) {}
}

const CASES: Case[] = [
  new Case("serves_the_index", "localhost", "GET / HTTP/1.1", "HTTP/1.1 200 OK"),
  new Case("serves_a_stylesheet", "localhost", "GET /style.css HTTP/1.1", "HTTP/1.1 200 OK"),
  new Case("missing_pages_are_not_found", "localhost", "GET /nope.html HTTP/1.1", "HTTP/1.1 404 "),
  new Case("directories_redirect_to_a_slash", "localhost", "GET /docs HTTP/1.1", "HTTP/1.1 301 "),
  new Case("writes_are_refused", "localhost", "POST / HTTP/1.1", "HTTP/1.1 405 "),
  new Case("the_files_host_serves_static_files", "files.localhost", "GET /style.css HTTP/1.1", "HTTP/1.1 200 OK"),
]

/**
 * The site's tests. Listing /tests names them; writing /tests/<name> dials
 * the site with that case's request, and once the server hangs up, answers
 * ok if the response began as it should, or an error saying how it began.
 */
class SiteTests extends SiteTestsServer {
  listTests(req: Request): void {
    for (let i = 0; i < CASES.length; i++) req.stream(Stat.file(CASES[i].name).payload())
    req.end()
  }

  writeTests(req: Request, name: string): void {
    for (let i = 0; i < CASES.length; i++) {
      const c = CASES[i]
      if (c.name != name) continue
      const request = c.line + "\r\nHost: " + c.host + "\r\n\r\n"
      net.createDial(new Payload(OCTETS, String.UTF8.encode(request)), new Check(req, c))
      return
    }
    req.fail(Status.NotFound)
  }
}

/** Collects a response until the server hangs up, then checks it. */
class Check extends Then {
  got: string = ""

  constructor(public req: Request, public c: Case) {
    super()
  }

  answer(res: Response): void {
    const body = res.body
    if (body !== null) this.got += body.asText()
    if (!res.done) return
    const line = this.got.split("\r\n")[0]
    if (!res.ok) this.req.fail(Status.Invalid, "dialing failed with " + res.status.code)
    else if (line.startsWith(this.c.want)) this.req.end()
    else this.req.fail(Status.Invalid, "expected `" + this.c.want + "...`, got `" + line + "`")
  }
}

run(new SiteTests())
