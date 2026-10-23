import { Errno, Request, Response, Stat, Then, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { Reader, Writer, decodeMessage, encodeMessage } from "othismo/bson"
import { Person, Record, TypesTestsServer, store } from "./TypesTests.oth"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Every field of `r`, as text, so that two records can be compared. */
function describe(r: Record): string {
  let grid: string[] = []
  for (let i = 0; i < r.grid.length; i++) grid.push("[" + r.grid[i].join(",") + "]")
  let team: string[] = []
  for (let i = 0; i < r.team.length; i++) team.push(person(r.team[i]))
  return "name=" + r.name + " count=" + r.count.toString() + " owner=" + person(r.owner) +
    " tags=[" + r.tags.join(",") + "] grid=[" + grid.join(",") + "] team=[" + team.join(",") + "]"
}

function person(p: Person): string {
  return "{" + p.name + " " + p.age.toString() + "}"
}

/** The record the image writes at startup, described. */
const STARTUP = "name=start count=7 owner={Ada 36} tags=[a,b] grid=[[1,2],[],[-3]] team=[{Bo 1}]"

/** A record with something in every field. */
function sample(): Record {
  return new Record("sample", -3, new Person("Grace", 85), ["x"], [[], [4]], [new Person("Al", 2), new Person("Cy", 3)])
}

/** "" if `got` is `want`, or how it isn't. */
function same(got: string, want: string): string {
  return got == want ? "" : "got `" + got + "`, want `" + want + "`"
}

/** A record survives being encoded and decoded in this module. */
function encodesAndDecodes(): string {
  const m = decodeMessage(encodeMessage(sample()))
  if (!(m instanceof Record)) return "decoded a " + m.typeName()
  return same(describe(changetype<Record>(m)), describe(sample()))
}

/**
 * Fields of types a reader doesn't ask for are stepped over: a null, an
 * array and a datetime, written by hand before the field it does.
 */
function skipsFieldsNoOneReads(): string {
  const w = new Writer()
  w.element(0x0A, "nothing", new ArrayBuffer(0))
  const array = new Uint8Array(12) // { "0": int32 5 }
  array[0] = 12
  array[4] = 0x10
  array[5] = 0x30 // "0"
  array[7] = 5
  w.element(0x04, "list", array.buffer)
  w.element(0x09, "when", new ArrayBuffer(8))
  w.string("after", "found")
  return same(new Reader(w.finish()).string("after"), "found")
}

/** One test: run in the module, or a check of the record the store holds. */
class Case {
  constructor(
    public name: string,
    public local: (() => string) | null,
    /** Write `sample()` to the store first, rather than reading the startup record. */
    public roundTrip: bool = false,
  ) {}
}

const CASES: Case[] = [
  new Case("encodes_and_decodes", encodesAndDecodes),
  new Case("skips_fields_no_one_reads", skipsFieldsNoOneReads),
  new Case("reads_the_startup_record", null),
  new Case("round_trips_through_the_store", null, true),
]

/**
 * The tests. Listing /tests names them; writing /tests/<name> runs one, and
 * answers ok if it passed, or an error saying why not.
 */
class TypesTests extends TypesTestsServer {
  listTests(req: Request): void {
    for (let i = 0; i < CASES.length; i++) req.stream(Stat.file(CASES[i].name).payload())
    req.end()
  }

  writeTests(req: Request, name: string): void {
    for (let i = 0; i < CASES.length; i++) {
      const c = CASES[i]
      if (c.name != name) continue
      const local = c.local
      if (local !== null) finish(req, local())
      else if (c.roundTrip) store.writeRecord(sample(), new Written(req))
      else store.readRecord(new Check(req, STARTUP))
      return
    }
    req.fail(Errno.NotFound)
  }
}

function finish(req: Request, problem: string): void {
  if (problem == "") req.end()
  else req.fail(Errno.Invalid, problem)
}

/** Once the sample is written, reads it back. */
class Written extends Then {
  constructor(public req: Request) {
    super()
  }

  answer(res: Response): void {
    if (!res.ok) finish(this.req, "writing failed with errno " + res.status.toString())
    else store.readRecord(new Check(this.req, describe(sample())))
  }
}

/** Checks the record read is the one described. */
class Check extends Then {
  constructor(public req: Request, public want: string) {
    super()
  }

  answer(res: Response): void {
    const r = Record.fromPayload(res.body)
    if (!res.ok) finish(this.req, "reading failed with errno " + res.status.toString())
    else if (r === null) finish(this.req, "the store's answer is not a Record")
    else finish(this.req, same(describe(r), this.want))
  }
}

run(new TypesTests())
