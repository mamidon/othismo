import { Status, Request, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { Record, StoreServer } from "./Store.oth"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Serves /record: keeps the last `Record` written, and reads it back. */
class Store extends StoreServer {
  record: Record | null = null

  writeRecord(req: Request, body: Record): void {
    this.record = body
    req.end()
  }

  readRecord(req: Request): void {
    const r = this.record
    if (r === null) req.fail(Status.NotFound, "no record has been written")
    else req.reply(r.toPayload())
  }
}

run(new Store())
