// The AssemblyScript side of the othismo runtime: message classes and
// their wire format. The namespace SDK (requests, replies, mounts) is in
// "othismo/ns", and the runtime's own message types are in "othismo/env".
//
// Wire format (see src/wire.rs): one BSON document per message whose first
// key is `$type` (the class name), followed by the class's fields.

const TYPE_KEY = "$type";

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
export class Opaque extends Message {
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
  if (msg instanceof Opaque) return (msg as Opaque).raw;
  const w = new Writer();
  w.string(TYPE_KEY, msg.typeName());
  msg.encode(w);
  return w.finish();
}

/** The message in `buf`, or an `Opaque` one if its type isn't registered. */
export function decodeMessage(buf: ArrayBuffer): Message {
  const r = new Reader(buf);
  const t = r.string(TYPE_KEY);
  if (!decoders.has(t)) return new Opaque(t, buf);
  return decoders.get(t)(r);
}

// ---------------------------------------------------------------------------
// BSON

const BSON_DOUBLE: u8 = 0x01;
const BSON_STRING: u8 = 0x02;
const BSON_BINARY: u8 = 0x05;
const BSON_BOOL: u8 = 0x08;
const BSON_INT32: u8 = 0x10;
const BSON_INT64: u8 = 0x12;

/** Builds one BSON document. Integer methods follow src/wire.rs's mapping. */
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

class Field {
  constructor(
    public tag: u8,
    public offset: usize, // absolute address of the value
  ) {}
}

/** Reads fields out of one BSON document. Missing or mistyped fields abort. */
export class Reader {
  private fields: Map<string, Field> = new Map<string, Field>();

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
      if (tag == BSON_DOUBLE || tag == BSON_INT64) p += 8;
      else if (tag == BSON_INT32) p += 4;
      else if (tag == BSON_BOOL) p += 1;
      else if (tag == BSON_STRING) p += 4 + <usize>load<u32>(p);
      else if (tag == BSON_BINARY) p += 5 + <usize>load<u32>(p);
      else abort("unsupported BSON element type in field " + name);
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
