// Generated from Store.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"
import { Payload } from "othismo"
import { Box, Message, Reader, Writer, bson, registerMessage } from "othismo/bson"

/** A field of each type. */
export class Record extends Message {
  name: string
  count: i32
  /** Another message. */
  owner: Person
  /** Lists, of anything a field can be. */
  tags: Array<string>
  grid: Array<Array<i32>>
  team: Array<Person>
  /** Maps, from strings to anything a field can be. */
  env: Map<string, string>
  teams: Map<string, Array<Person>>
  /** Optional fields, which a record may leave out. */
  nick: string | null
  rank: Box<i32> | null
  boss: Person | null

  constructor(name: string, count: i32, owner: Person, tags: Array<string>, grid: Array<Array<i32>>, team: Array<Person>, env: Map<string, string>, teams: Map<string, Array<Person>>, nick: string | null, rank: Box<i32> | null, boss: Person | null) {
    super()
    this.name = name
    this.count = count
    this.owner = owner
    this.tags = tags
    this.grid = grid
    this.team = team
    this.env = env
    this.teams = teams
    this.nick = nick
    this.rank = rank
    this.boss = boss
  }

  /** The `Record` in `p`, or null if it holds none. */
  static fromPayload(p: Payload | null): Record | null {
    const m = bson.decode(p)
    return m instanceof Record ? changetype<Record>(m) : null
  }
  toPayload(): Payload { return bson.payload(this) }

  typeName(): string { return "Record" }
  encode(w: Writer): void {
    w.string("name", this.name)
    w.i32("count", this.count)
    w.doc("owner", this.owner)
    w.array("tags", __write_list_string(this.tags))
    w.array("grid", __write_list_list_i32(this.grid))
    w.array("team", __write_list_Person(this.team))
    w.map("env", __write_map_string(this.env))
    w.map("teams", __write_map_list_Person(this.teams))
    if (this.nick !== null) w.string("nick", this.nick!)
    if (this.rank !== null) w.i32("rank", this.rank!.value)
    if (this.boss !== null) w.doc("boss", this.boss!)
  }
  static decode(r: Reader): Record { return new Record(r.string("name"), r.i32("count"), Person.decode(r.doc("owner")), __read_list_string(r.array("tags")), __read_list_list_i32(r.array("grid")), __read_list_Person(r.array("team")), __read_map_string(r.map("env")), __read_map_list_Person(r.map("teams")), r.has("nick") ? <string | null>r.string("nick") : null, r.has("rank") ? <Box<i32> | null>new Box<i32>(r.i32("rank")) : null, r.has("boss") ? <Person | null>Person.decode(r.doc("boss")) : null) }
}
registerMessage("Record", (r: Reader): Message => Record.decode(r))

/** Someone. */
export class Person extends Message {
  name: string
  age: u8

  constructor(name: string, age: u8) {
    super()
    this.name = name
    this.age = age
  }

  /** The `Person` in `p`, or null if it holds none. */
  static fromPayload(p: Payload | null): Person | null {
    const m = bson.decode(p)
    return m instanceof Person ? changetype<Person>(m) : null
  }
  toPayload(): Payload { return bson.payload(this) }

  typeName(): string { return "Person" }
  encode(w: Writer): void {
    w.string("name", this.name)
    w.i32("age", <i32>this.age)
  }
  static decode(r: Reader): Person { return new Person(r.string("name"), <u8>r.i64("age")) }
}
registerMessage("Person", (r: Reader): Message => Person.decode(r))

function __read_list_Person(r: Reader): Array<Person> {
  const keys = r.keys()
  const out = new Array<Person>()
  for (let i = 0; i < keys.length; i++) out.push(Person.decode(r.doc(keys[i])))
  return out
}

function __read_list_i32(r: Reader): Array<i32> {
  const keys = r.keys()
  const out = new Array<i32>()
  for (let i = 0; i < keys.length; i++) out.push(r.i32(keys[i]))
  return out
}

function __read_list_list_i32(r: Reader): Array<Array<i32>> {
  const keys = r.keys()
  const out = new Array<Array<i32>>()
  for (let i = 0; i < keys.length; i++) out.push(__read_list_i32(r.array(keys[i])))
  return out
}

function __read_list_string(r: Reader): Array<string> {
  const keys = r.keys()
  const out = new Array<string>()
  for (let i = 0; i < keys.length; i++) out.push(r.string(keys[i]))
  return out
}

function __read_map_list_Person(r: Reader): Map<string, Array<Person>> {
  const keys = r.keys()
  const out = new Map<string, Array<Person>>()
  for (let i = 0; i < keys.length; i++) out.set(keys[i], __read_list_Person(r.array(keys[i])))
  return out
}

function __read_map_string(r: Reader): Map<string, string> {
  const keys = r.keys()
  const out = new Map<string, string>()
  for (let i = 0; i < keys.length; i++) out.set(keys[i], r.string(keys[i]))
  return out
}

function __write_list_Person(v: Array<Person>): Writer {
  const w = new Writer()
  for (let i = 0; i < v.length; i++) w.doc(i.toString(), v[i])
  return w
}

function __write_list_i32(v: Array<i32>): Writer {
  const w = new Writer()
  for (let i = 0; i < v.length; i++) w.i32(i.toString(), v[i])
  return w
}

function __write_list_list_i32(v: Array<Array<i32>>): Writer {
  const w = new Writer()
  for (let i = 0; i < v.length; i++) w.array(i.toString(), __write_list_i32(v[i]))
  return w
}

function __write_list_string(v: Array<string>): Writer {
  const w = new Writer()
  for (let i = 0; i < v.length; i++) w.string(i.toString(), v[i])
  return w
}

function __write_map_list_Person(v: Map<string, Array<Person>>): Writer {
  const w = new Writer()
  const keys = v.keys()
  for (let i = 0; i < keys.length; i++) w.array(keys[i], __write_list_Person(v.get(keys[i])))
  return w
}

function __write_map_string(v: Map<string, string>): Writer {
  const w = new Writer()
  const keys = v.keys()
  for (let i = 0; i < keys.length; i++) w.string(keys[i], v.get(keys[i]))
  return w
}

/**
 * Keeps the last `Record` written to it, to read back: a payload of each of
 * the BSON format's field types, sent and received.
 */
export class StoreServer extends Server {
  /**
   * The last record written.
   *
   * `/record write Record`
   */
  writeRecord(req: Request, body: Record): void { req.fail(Errno.NotSupported) }

  /**
   * The last record written.
   *
   * `/record read -> Record`
   */
  readRecord(req: Request): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/record", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { const body = Record.fromPayload(req.body); if (body !== null) { this.writeRecord(req, body); return } }
      if (req.verb == Verb.Read) { this.readRecord(req); return }
      if (req.verb == Verb.Write) { req.fail(Errno.Invalid, "expected a Record"); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
  }
}
