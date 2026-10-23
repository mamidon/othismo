// Generated from TypesTests.oth by `othismo bindings`. Edit the header, not this file.

import { Call, Errno, Payload, Request, Server, Then, Verb, ns, payloadOf, route, textOf, under } from "othismo"
import { Payload } from "othismo"
import { Message, Reader, Writer, bson, registerMessage } from "othismo/bson"

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

  constructor(name: string, count: i32, owner: Person, tags: Array<string>, grid: Array<Array<i32>>, team: Array<Person>, env: Map<string, string>, teams: Map<string, Array<Person>>) {
    super()
    this.name = name
    this.count = count
    this.owner = owner
    this.tags = tags
    this.grid = grid
    this.team = team
    this.env = env
    this.teams = teams
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
  }
  static decode(r: Reader): Record { return new Record(r.string("name"), r.i32("count"), Person.decode(r.doc("owner")), __read_list_string(r.array("tags")), __read_list_list_i32(r.array("grid")), __read_list_Person(r.array("team")), __read_map_string(r.map("env")), __read_map_list_Person(r.map("teams"))) }
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
  writeTests(req: Request, name: string): void { req.fail(Errno.NotSupported) }

  /**
   * The tests, a stat record each.
   *
   * `/tests list`
   */
  listTests(req: Request): void { req.fail(Errno.NotSupported) }

  serve(req: Request): void {
    const m0 = route("/tests/{name}", req.path)
    if (m0 !== null) {
      if (req.verb == Verb.Write) { this.writeTests(req, m0![0]); return }
      req.fail(Errno.NotSupported)
      return
    }
    const m1 = route("/tests", req.path)
    if (m1 !== null) {
      if (req.verb == Verb.List) { this.listTests(req); return }
      req.fail(Errno.NotSupported)
      return
    }
    req.fail(Errno.NotFound)
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
