import { Errno, Request, Response, Stat, Then, run, rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel } from "othismo"
import { Box, Reader, Uuid, Writer, decodeMessage, encodeMessage } from "othismo/bson"
import { Color, Person, Record, TypesTestsServer, nameOfColor, store } from "./TypesTests.oth"

export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

/** Every field of `r`, as text, so that two records can be compared. */
function describe(r: Record): string {
  let grid: string[] = []
  for (let i = 0; i < r.grid.length; i++) grid.push("[" + r.grid[i].join(",") + "]")
  let team: string[] = []
  for (let i = 0; i < r.team.length; i++) team.push(person(r.team[i]))
  return "name=" + r.name + " count=" + r.count.toString() + " born=" + r.born.getTime().toString() + " id=" + r.id.toString() +
    " owner=" + person(r.owner) +
    " tags=[" + r.tags.join(",") + "] grid=[" + grid.join(",") + "] team=[" + team.join(",") + "]" +
    " env={" + env(r.env) + "} teams={" + teams(r.teams) + "}" +
    " nick=" + (r.nick === null ? "-" : r.nick!) + " rank=" + (r.rank === null ? "-" : r.rank!.value.toString()) +
    " boss=" + (r.boss === null ? "-" : person(r.boss!)) +
    " color=" + nameOfColor(r.color) + " palette=[" + r.palette.map<string>((c: Color): string => nameOfColor(c)).join(",") + "]" +
    " fav=" + (r.fav === null ? "-" : nameOfColor(r.fav!.value))
}

function env(m: Map<string, string>): string {
  const keys = m.keys()
  let out: string[] = []
  for (let i = 0; i < keys.length; i++) out.push(keys[i] + ":" + m.get(keys[i]))
  return out.join(",")
}

function teams(m: Map<string, Array<Person>>): string {
  const keys = m.keys()
  let out: string[] = []
  for (let i = 0; i < keys.length; i++) {
    const team = m.get(keys[i])
    let people: string[] = []
    for (let j = 0; j < team.length; j++) people.push(person(team[j]))
    out.push(keys[i] + ":[" + people.join(",") + "]")
  }
  return out.join(",")
}

function person(p: Person): string {
  return "{" + p.name + " " + p.age.toString() + "}"
}

/** The record the image writes at startup, described. */
const STARTUP = "name=start count=7 born=1792758896789 id=123e4567-e89b-12d3-a456-426614174000 owner={Ada 36} tags=[a,b] grid=[[1,2],[],[-3]] team=[{Bo 1}] env={HOME:/home,Content-Type:text/plain} teams={red:[{Cy 2}],blue:[]} nick=st rank=2 boss=- color=green palette=[blue,red] fav=-"

/** A record with something in every field. */
function sample(): Record {
  const env = new Map<string, string>()
  env.set("b", "2")
  env.set("a", "1")
  const teams = new Map<string, Array<Person>>()
  teams.set("solo", [new Person("Di", 4)])
  return new Record("sample", -3, new Date(1000), Uuid.parse("00000000-0000-0000-0000-0000000000ff")!, new Person("Grace", 85), ["x"], [[], [4]], [new Person("Al", 2), new Person("Cy", 3)], env, teams,
    null, new Box<i32>(-1), new Person("Ed", 5), Color.blue, [], new Box<Color>(Color.red))
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
 * array and a datetime, written by hand before the field it does. A null
 * is as good as missing.
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
  const r = new Reader(w.finish())
  if (r.has("nothing") || r.has("missing") || !r.has("after")) return "a null field or a missing one should not count as there, and one that is should"
  return same(r.string("after"), "found")
}

/** UUIDs are parsed and written as people write them. */
function writesUuids(): string {
  const id = Uuid.parse("123E4567-e89b-12d3-a456-42661417400a")
  if (id === null) return "a UUID didn't parse"
  if (Uuid.parse("123e4567-e89b-12d3-a456-42661417400") !== null) return "a short UUID parsed"
  if (Uuid.parse("123e4567+e89b-12d3-a456-42661417400a") !== null) return "a UUID without its dashes parsed"
  if (Uuid.parse("123e4567-e89b-12d3-a456-42661417400g") !== null) return "a UUID with a g in it parsed"
  return same(id!.toString(), "123e4567-e89b-12d3-a456-42661417400a")
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
  new Case("writes_uuids", writesUuids),
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
