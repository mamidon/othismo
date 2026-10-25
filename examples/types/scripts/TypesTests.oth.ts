// Generated from TypesTests.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Payload, Request, Server, Status, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"
import { Payload } from "othismo"
import { Box, Message, Reader, Uuid, Writer, bson, registerMessage } from "othismo/bson"

/**
 * A message the store has no class for, to hold in an `any` field and
 * pass on as it came.
 */
export class Note extends Message {
  text: string

  constructor(text: string) {
    super()
    this.text = text
  }

  /** The `Note` in `p`, or null if it holds none. */
  static fromPayload(p: Payload | null): Note | null {
    const m = bson.decode(p)
    return m instanceof Note ? changetype<Note>(m) : null
  }
  toPayload(): Payload { return bson.payload(this) }

  typeName(): string { return "Note" }
  encode(w: Writer): void { w.string("text", this.text) }
  static decode(r: Reader): Note { return new Note(r.string("text")) }
}
registerMessage("Note", (r: Reader): Message => Note.decode(r))

/** A field of each type. */
export class Record extends Message {
  name: string
  count: i32
  /** A moment, and an id. */
  born: Date
  id: Uuid
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
  /** Enums, alone, in lists, and optional. */
  color: Color
  palette: Array<Color>
  fav: Box<Color> | null
  /** Any message, which says its own type. */
  extra: Message
  extras: Array<Message>

  constructor(name: string, count: i32, born: Date, id: Uuid, owner: Person, tags: Array<string>, grid: Array<Array<i32>>, team: Array<Person>, env: Map<string, string>, teams: Map<string, Array<Person>>, nick: string | null, rank: Box<i32> | null, boss: Person | null, color: Color, palette: Array<Color>, fav: Box<Color> | null, extra: Message, extras: Array<Message>) {
    super()
    this.name = name
    this.count = count
    this.born = born
    this.id = id
    this.owner = owner
    this.tags = tags
    this.grid = grid
    this.team = team
    this.env = env
    this.teams = teams
    this.nick = nick
    this.rank = rank
    this.boss = boss
    this.color = color
    this.palette = palette
    this.fav = fav
    this.extra = extra
    this.extras = extras
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
    w.time("born", this.born)
    w.uuid("id", this.id)
    w.doc("owner", this.owner)
    w.array("tags", __write_list_string(this.tags))
    w.array("grid", __write_list_list_i32(this.grid))
    w.array("team", __write_list_Person(this.team))
    w.map("env", __write_map_string(this.env))
    w.map("teams", __write_map_list_Person(this.teams))
    if (this.nick !== null) w.string("nick", this.nick!)
    if (this.rank !== null) w.i32("rank", this.rank!.value)
    if (this.boss !== null) w.doc("boss", this.boss!)
    w.string("color", nameOfColor(this.color))
    w.array("palette", __write_list_Color(this.palette))
    if (this.fav !== null) w.string("fav", nameOfColor(this.fav!.value))
    w.message("extra", this.extra)
    w.array("extras", __write_list_Message(this.extras))
  }
  static decode(r: Reader): Record { return new Record(r.string("name"), r.i32("count"), r.time("born"), r.uuid("id"), Person.decode(r.doc("owner")), __read_list_string(r.array("tags")), __read_list_list_i32(r.array("grid")), __read_list_Person(r.array("team")), __read_map_string(r.map("env")), __read_map_list_Person(r.map("teams")), r.has("nick") ? <string | null>r.string("nick") : null, r.has("rank") ? <Box<i32> | null>new Box<i32>(r.i32("rank")) : null, r.has("boss") ? <Person | null>Person.decode(r.doc("boss")) : null, __parse_Color(r.string("color")), __read_list_Color(r.array("palette")), r.has("fav") ? <Box<Color> | null>new Box<Color>(__parse_Color(r.string("fav"))) : null, r.message("extra"), __read_list_Message(r.array("extras"))) }
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

/** A color, carried as its name. */
export enum Color {
  red,
  green,
  blue,
}

/** The name of a `Color`, as the wire carries it. */
export function nameOfColor(v: Color): string {
  switch (v) {
    case Color.red: return "red"
    case Color.green: return "green"
    case Color.blue: return "blue"
  }
  abort("not a Color")
  return ""
}

function __parse_Color(s: string): Color {
  if (s == "red") return Color.red
  if (s == "green") return Color.green
  if (s == "blue") return Color.blue
  abort("`" + s + "` is not a Color")
  return 0
}

function __read_list_Color(r: Reader): Array<Color> {
  const keys = r.keys()
  const out = new Array<Color>()
  for (let i = 0; i < keys.length; i++) out.push(__parse_Color(r.string(keys[i])))
  return out
}

function __read_list_Message(r: Reader): Array<Message> {
  const keys = r.keys()
  const out = new Array<Message>()
  for (let i = 0; i < keys.length; i++) out.push(r.message(keys[i]))
  return out
}

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

function __write_list_Color(v: Array<Color>): Writer {
  const w = new Writer()
  for (let i = 0; i < v.length; i++) w.string(i.toString(), nameOfColor(v[i]))
  return w
}

function __write_list_Message(v: Array<Message>): Writer {
  const w = new Writer()
  for (let i = 0; i < v.length; i++) w.message(i.toString(), v[i])
  return w
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
 * Tests the BSON format's field types in AssemblyScript: each test encodes
 * and decodes in the module itself, or round-trips a record through the
 * store, which the image wrote one to at startup.
 */
export class TypesTestsServer extends Server {
  /**
   * Runs a test, and answers `ok` if it passed, or an error, conventionally
   * `invalid`, whose description says why it didn't.
   *
   * `/tests/{name} write`
   */
  writeTests(req: Request, name: string): void { req.fail(Status.NotSupported) }

  /**
   * The tests, a stat record each.
   *
   * `/tests list`
   */
  listTests(req: Request): void { req.fail(Status.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/tests/{name}", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { this.writeTests(req, m0![0]); return }
      req.fail(Status.NotSupported)
      return
    }
    const m1 = route("/tests", req.path)
    if (m1 !== null) {
      if (req.verb == Verb.List) { this.listTests(req); return }
      req.fail(Status.NotSupported)
      return
    }
    req.fail(Status.NotFound)
  }
}

/** The store to write records to and read them back from. */
export class StoreClient {
  constructor(public at: string) {}

  /** `/record write Record` */
  writeRecord(body: Record, then: Then | null = null): Call {
    return ns.call(Verb.Write, under(this.at, "/record"), body.toPayload(), then)
  }

  /** `/record read -> Record` */
  readRecord(then: Then): Call {
    return ns.call(Verb.Read, under(this.at, "/record"), null, then)
  }
}

/** The store to write records to and read them back from. */
export const store = new StoreClient("/store")
