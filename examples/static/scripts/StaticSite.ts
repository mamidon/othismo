import { Flag, Payload, Request, Server, Errno, Verb, isSafe, ns, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/**
 * A site served from /files, which it can only read. A path ending in "/"
 * means its index.html. It never sees the files' contents: it forwards each
 * request, and the answer goes straight back to whoever asked.
 *
 * Its tree also has /hits, a synthetic file counting the requests it has
 * forwarded, and /static, where /files is exposed as is: requests there
 * don't reach this code at all.
 */
class StaticSite extends Server {
  hits: u64 = 0

  start(): void {
    ns.expose("/static", "/files", Flag.ReadOnly)
  }

  serve(req: Request): void {
    if (req.path == "/hits") {
      if (req.verb == Verb.Read) req.reply(Payload.text(this.hits.toString() + "\n", "text/plain; charset=utf-8"))
      else req.fail(Errno.NotSupported)
      return
    }
    if (!isSafe(req.verb)) {
      req.fail(Errno.ReadOnly)
      return
    }
    this.hits++
    req.forward("/files" + (req.path.endsWith("/") ? req.path + "index.html" : req.path))
  }
}

run(new StaticSite())
