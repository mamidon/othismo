// BSON messages, the first payload format: each one is a BSON document
// whose first key is `$type` (the class name), followed by the class's
// fields (see crates/format-bson/src/wire.rs). Its content type is `application/bson`, and the
// document names its own type. Bindings for a header's message types are
// generated from it, and register themselves here.

import { Payload } from "./payload"

const TYPE_KEY = "$type";

/** The content type of a BSON payload. */
export const BSON = "application/bson";

// ---------------------------------------------------------------------------
// Messages

/** Base class for every message. Subclasses override both members. */
export class Message {
  /** The class name; must match the name in its header. */
  typeName(): string {
    abort("Message subclass does not override typeName()");
    return "";
  }

  /** Writes each field with the matching `Writer` method. */
  encode(w: Writer): void {}
}

/**
 * A message of a type this module has no decoder for. It can't be read, but
 * it can be passed on unchanged: encoding it gives back the same bytes.
 */
export class Unknown extends Message {
  constructor(public type: string, public raw: ArrayBuffer) {
    super();
  }

  typeName(): string { return this.type; }
}

type Decoder = (r: Reader) => Message;

const decoders = new Map<string, Decoder>();

/** Makes `typeName` receivable. Call once per message class, at top level. */
export function registerMessage(typeName: string, decode: Decoder): void {
  decoders.set(typeName, decode);
}

/** The wire bytes for `msg`. */
export function encodeMessage(msg: Message): ArrayBuffer {
  if (msg instanceof Unknown) return (msg as Unknown).raw;
  const w = new Writer();
  w.string(TYPE_KEY, msg.typeName());
  msg.encode(w);
  return w.finish();
}

/** The message in `buf`, or an `Unknown` one if its type isn't registered. */
export function decodeMessage(buf: ArrayBuffer): Message {
  const r = new Reader(buf);
  const t = r.string(TYPE_KEY);
  if (!decoders.has(t)) return new Unknown(t, buf);
  return decoders.get(t)(r);
}

export namespace bson {
  /** `msg` as an `application/bson` payload. */
  export function payload(msg: Message): Payload {
    return new Payload(BSON, encodeMessage(msg));
  }

  /** The message in `p`, or null if it isn't a BSON payload. */
  export function decode(p: Payload | null): Message | null {
    if (p === null || p.type != BSON) return null;
    return decodeMessage(p.bytes);
  }
}

// ---------------------------------------------------------------------------
// BSON

const BSON_DOUBLE: u8 = 0x01;
const BSON_STRING: u8 = 0x02;
const BSON_DOCUMENT: u8 = 0x03;
const BSON_ARRAY: u8 = 0x04;
const BSON_BINARY: u8 = 0x05;
const BSON_BOOL: u8 = 0x08;
const BSON_INT32: u8 = 0x10;
const BSON_INT64: u8 = 0x12;

/** Builds one BSON document. Integer methods follow crates/format-bson/src/wire.rs's mapping. */
export class Writer {
  private buf: ArrayBuffer = new ArrayBuffer(64);
  private len: i32 = 4; // room for the document length, patched by finish()

  string(name: string, v: string): void {
    this.key(BSON_STRING, name);
    const utf8 = String.UTF8.encode(v);
    this.u32(<u32>utf8.byteLength + 1);
    this.raw(changetype<usize>(utf8), utf8.byteLength);
    this.u8(0);
  }

  bool(name: string, v: bool): void {
    this.key(BSON_BOOL, name);
    this.u8(v ? 1 : 0);
  }

  /** For i8, i16, i32, u8 and u16 fields. */
  i32(name: string, v: i32): void {
    this.key(BSON_INT32, name);
    this.u32(<u32>v);
  }

  /** For i64, u32 and u64 fields. */
  i64(name: string, v: i64): void {
    this.key(BSON_INT64, name);
    this.u64(<u64>v);
  }

  /** For f32 and f64 fields. */
  f64(name: string, v: f64): void {
    this.key(BSON_DOUBLE, name);
    this.u64(reinterpret<u64>(v));
  }

  /** For ArrayBuffer fields. */
  bytes(name: string, v: ArrayBuffer): void {
    this.key(BSON_BINARY, name);
    this.u32(<u32>v.byteLength);
    this.u8(0); // generic subtype
    this.raw(changetype<usize>(v), v.byteLength);
  }

  /** For message fields: `m`'s fields, as an embedded document. */
  doc(name: string, m: Message): void {
    const sub = new Writer();
    m.encode(sub);
    this.element(BSON_DOCUMENT, name, sub.finish());
  }

  /** For list fields: `items`, whose keys are "0", "1", ..., as an array. */
  array(name: string, items: Writer): void {
    this.element(BSON_ARRAY, name, items.finish());
  }

  /** For map fields: `entries`, by their keys, as a document. */
  map(name: string, entries: Writer): void {
    this.element(BSON_DOCUMENT, name, entries.finish());
  }

  /** An element of any BSON type, its value's bytes as BSON lays them out. */
  element(tag: u8, name: string, value: ArrayBuffer): void {
    this.key(tag, name);
    this.raw(changetype<usize>(value), value.byteLength);
  }

  finish(): ArrayBuffer {
    this.u8(0);
    const out = this.buf.slice(0, this.len);
    store<u32>(changetype<usize>(out), <u32>this.len);
    return out;
  }

  private key(tag: u8, name: string): void {
    this.u8(tag);
    const utf8 = String.UTF8.encode(name);
    this.raw(changetype<usize>(utf8), utf8.byteLength);
    this.u8(0);
  }

  private reserve(n: i32): usize {
    if (this.len + n > this.buf.byteLength) {
      let cap = this.buf.byteLength * 2;
      while (cap < this.len + n) cap *= 2;
      const grown = new ArrayBuffer(cap);
      memory.copy(changetype<usize>(grown), changetype<usize>(this.buf), <usize>this.len);
      this.buf = grown;
    }
    const at = changetype<usize>(this.buf) + <usize>this.len;
    this.len += n;
    return at;
  }

  private raw(src: usize, n: i32): void {
    memory.copy(this.reserve(n), src, <usize>n);
  }

  private u8(v: u8): void {
    store<u8>(this.reserve(1), v);
  }

  private u32(v: u32): void {
    store<u32>(this.reserve(4), v);
  }

  private u64(v: u64): void {
    store<u64>(this.reserve(8), v);
  }
}

/**
 * The size of the value of an element of type `tag` at `p`, so that one of
 * a type no message field has can be stepped over: a newer sender may add
 * such fields. Only a type whose size can't be known aborts.
 */
function size(tag: u8, p: usize, end: usize, name: string): usize {
  switch (tag) {
    case 0x06: case 0x0A: case 0x7F: case 0xFF: return 0; // undefined, null, max key, min key
    case BSON_BOOL: return 1;
    case BSON_INT32: return 4;
    case BSON_DOUBLE: case 0x09: case 0x11: case BSON_INT64: return 8; // and datetime, timestamp
    case 0x07: return 12; // object id
    case 0x13: return 16; // decimal128
    case BSON_STRING: case 0x0D: case 0x0E: return 4 + <usize>load<u32>(p); // and code, symbol
    case BSON_BINARY: return 5 + <usize>load<u32>(p);
    case BSON_DOCUMENT: case BSON_ARRAY: case 0x0F: return <usize>load<u32>(p); // document, array, code with scope
    case 0x0C: return 4 + <usize>load<u32>(p) + 12; // db pointer
    case 0x0B: { // regex: two cstrings
      let q = p;
      for (let n = 0; n < 2; n++) {
        while (q < end && load<u8>(q) != 0) q++;
        q++;
      }
      return q - p;
    }
  }
  abort("unknown BSON element type in field " + name);
  return 0;
}

class Field {
  constructor(
    public tag: u8,
    public offset: usize, // absolute address of the value
  ) {}
}

/**
 * Reads fields out of one BSON document. Missing or mistyped fields abort;
 * fields no one asks for are skipped, whatever their type.
 */
export class Reader {
  private fields: Map<string, Field> = new Map<string, Field>();
  private names: string[] = [];

  constructor(private buf: ArrayBuffer) {
    const base = changetype<usize>(buf);
    const end = base + <usize>buf.byteLength;
    if (buf.byteLength < 5 || <usize>load<u32>(base) != <usize>buf.byteLength) abort("malformed BSON document");
    let p = base + 4;
    while (p < end && load<u8>(p) != 0) {
      const tag = load<u8>(p);
      p++;
      let q = p;
      while (q < end && load<u8>(q) != 0) q++;
      const name = String.UTF8.decodeUnsafe(p, q - p);
      p = q + 1;
      this.fields.set(name, new Field(tag, p));
      this.names.push(name);
      p += size(tag, p, end, name);
    }
  }

  string(name: string): string {
    const f = this.field(name, BSON_STRING);
    return String.UTF8.decodeUnsafe(f.offset + 4, <usize>load<u32>(f.offset) - 1);
  }

  bool(name: string): bool {
    return load<u8>(this.field(name, BSON_BOOL).offset) != 0;
  }

  /** Any integer field, widened to i64. Narrow it with a cast. */
  i64(name: string): i64 {
    const f = this.get(name);
    if (f.tag == BSON_INT32) return <i64>load<i32>(f.offset);
    if (f.tag == BSON_INT64) return load<i64>(f.offset);
    abort("field " + name + " is not an integer");
    return 0;
  }

  i32(name: string): i32 {
    return <i32>this.i64(name);
  }

  f64(name: string): f64 {
    return load<f64>(this.field(name, BSON_DOUBLE).offset);
  }

  /** A copy of a binary field's bytes. */
  bytes(name: string): ArrayBuffer {
    const f = this.field(name, BSON_BINARY);
    const len = <i32>load<u32>(f.offset);
    const out = new ArrayBuffer(len);
    memory.copy(changetype<usize>(out), f.offset + 5, <usize>len);
    return out;
  }

  /** A message field's fields, for its type's `decode`. */
  doc(name: string): Reader {
    return new Reader(this.embedded(this.field(name, BSON_DOCUMENT)));
  }

  /** A list field's items, each under its index: "0", "1", .... */
  array(name: string): Reader {
    return new Reader(this.embedded(this.field(name, BSON_ARRAY)));
  }

  /** A map field's entries, each under its key. */
  map(name: string): Reader {
    return this.doc(name);
  }

  /** Every field's name, in order. */
  keys(): string[] {
    return this.names.slice();
  }

  /** A copy of an embedded document. */
  private embedded(f: Field): ArrayBuffer {
    const len = <i32>load<u32>(f.offset);
    const out = new ArrayBuffer(len);
    memory.copy(changetype<usize>(out), f.offset, <usize>len);
    return out;
  }

  private get(name: string): Field {
    if (!this.fields.has(name)) abort("message is missing field " + name);
    return this.fields.get(name);
  }

  private field(name: string, tag: u8): Field {
    const f = this.get(name);
    if (f.tag != tag) abort("field " + name + " has the wrong BSON type");
    return f;
  }
}
