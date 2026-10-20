// The AssemblyScript side of the real_talk runtime.
//
// An object module imports this library to talk to the host. An object never
// knows who it talks to. It can do two things:
//
//   emit(msg)        send into the image's routing; the image decides where
//   from.reply(msg)  answer whoever sent the message being handled
//
// Those are the module's only capabilities (host imports `real_talk.emit` and
// `real_talk.reply`). Everything else here is plain AssemblyScript: BSON
// encoding/decoding and receive dispatch.
//
// Wire format (see src/wire.rs): one BSON document per message whose first
// key is `$type` (the class name), followed by the class's fields.

// ---------------------------------------------------------------------------
// Host interface

// @ts-ignore: decorator
@external("real_talk", "emit")
declare function host_emit(ptr: usize, len: usize): void;

// @ts-ignore: decorator
@external("real_talk", "reply")
declare function host_reply(sender: u32, ptr: usize, len: usize): void;

const TYPE_KEY = "$type";

// ---------------------------------------------------------------------------
// Messages

/** Base class for every message. Subclasses override both members. */
export class Message {
  /** The class name; must match the name in messages.ts. */
  typeName(): string {
    abort("Message subclass does not override typeName()");
    return "";
  }

  /** Writes each field with the matching `Writer` method. */
  encode(w: Writer): void {}
}

type Decoder = (r: Reader) => Message;

const decoders = new Map<string, Decoder>();

/** Makes `typeName` receivable. Call once per message class, at top level. */
export function registerMessage(typeName: string, decode: Decoder): void {
  decoders.set(typeName, decode);
}

function encodeMessage(msg: Message): ArrayBuffer {
  const w = new Writer();
  w.string(TYPE_KEY, msg.typeName());
  msg.encode(w);
  return w.finish();
}

// ---------------------------------------------------------------------------
// Objects

/** Sends `msg` wherever the image routes this object's messages of its type. */
export function emit(msg: Message): void {
  const buf = encodeMessage(msg);
  host_emit(changetype<usize>(buf), <usize>buf.byteLength);
}

/**
 * Whoever sent the message being handled. Keep it to reply later; it stays
 * valid for the life of the object. Replies to the image's own startup
 * messages go nowhere.
 */
export class Sender {
  constructor(private handle: u32) {}

  reply(msg: Message): void {
    const buf = encodeMessage(msg);
    host_reply(this.handle, changetype<usize>(buf), <usize>buf.byteLength);
  }
}

/** Base class for objects. Override `receive` and test `msg instanceof T`. */
export class Receiver {
  receive(msg: Message, from: Sender): void {}
}

let receiver: Receiver | null = null;

/** Registers the module's object. Call once, at top level. */
export function bind(r: Receiver): void {
  receiver = r;
}

// ---------------------------------------------------------------------------
// Host entry points. Object modules re-export these:
//   export { rt_inbox, rt_deliver }

let inbox = new ArrayBuffer(0);

/** Allocates a buffer the host writes the next message into. */
export function rt_inbox(len: i32): usize {
  inbox = new ArrayBuffer(len);
  return changetype<usize>(inbox);
}

/** Decodes the message in the inbox and hands it to the bound object. */
export function rt_deliver(sender: u32): void {
  const r = new Reader(inbox);
  const t = r.string(TYPE_KEY);
  if (!decoders.has(t)) abort("no decoder registered for message type " + t);
  const msg = decoders.get(t)(r);
  const obj = receiver;
  if (obj === null) abort("no object bound; call bind(new YourObject())");
  else obj.receive(msg, new Sender(sender));
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
