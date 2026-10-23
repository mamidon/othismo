// The namespace SDK: how an object serves its tree and talks to its view.
//
// It is part of "othismo". An object module extends `Server`, calls
// `run(new MyServer())` at top level, and re-exports the host's entry points:
//
//   export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }
//
// Requests are made with the `ns` functions and answered through `Then`
// objects; requests made of us arrive at `Server.serve`. See
// docs/namespace.md for the model.

import { Payload } from "./payload"

// ---------------------------------------------------------------------------
// Host interface

// @ts-ignore: decorator
@external("othismo", "call")
declare function host_call(verb: i32, path: usize, pathLen: i32, type: usize, typeLen: i32, body: usize, bodyLen: i32, expect: i32): u32;

// @ts-ignore: decorator
@external("othismo", "reply")
declare function host_reply(req: u32, type: usize, typeLen: i32, body: usize, bodyLen: i32, done: i32): void;

// @ts-ignore: decorator
@external("othismo", "fail")
declare function host_fail(req: u32, errno: i32, desc: usize, descLen: i32): void;

// @ts-ignore: decorator
@external("othismo", "forward")
declare function host_forward(req: u32, path: usize, pathLen: i32): void;

// @ts-ignore: decorator
@external("othismo", "cancel")
declare function host_cancel(ticket: u32): void;

// @ts-ignore: decorator
@external("othismo", "bind")
declare function host_bind(src: usize, srcLen: i32, at: usize, atLen: i32, flags: i32): i32;

// @ts-ignore: decorator
@external("othismo", "unmount")
declare function host_unmount(at: usize, atLen: i32): i32;

// @ts-ignore: decorator
@external("othismo", "expose")
declare function host_expose(at: usize, atLen: i32, src: usize, srcLen: i32, flags: i32): i32;

// ---------------------------------------------------------------------------
// Verbs, errors, flags (numbered as in src/ns.rs)

export enum Verb { Read, Stat, List, Watch, Write, Create, Remove }

/** Safe verbs never change anything; read-only mounts let only these through. */
export function isSafe(v: Verb): bool {
  return v < Verb.Write;
}

/** How a request went: Ok, or an error. Each error's number is its errno. */
export enum Errno { Ok, NotFound, ReadOnly, NotSupported, IsDirectory, NotDirectory, Exists, Invalid, Io, Forbidden, UnsupportedType }

export enum Flag { None = 0, ReadOnly = 1 }

/** A request's flags, as `rt_serve` passes them. */
const OPERATOR: i32 = 1;

// ---------------------------------------------------------------------------
// Serving

/** Requests we are serving, by id, so a cancel can find them. */
const open = new Map<u32, Request>();

/** A request made of our served tree. Keep it to answer later if needed. */
export class Request {
  /** The caller gave up; anything we reply is dropped. */
  cancelled: bool = false;
  finished: bool = false;

  constructor(
    public id: u32,
    public verb: Verb,
    /** Relative to our root, e.g. "/index.html". */
    public path: string,
    public body: Payload | null,
    /** Whoever runs the image made it: its startup requests, never an object. */
    public operator: bool = false,
  ) {}

  /** Ok, and the last reply. */
  reply(body: Payload | null): void { this.send(body, true); }

  /** Ok, with more to come. */
  stream(body: Payload): void { this.send(body, false); }

  /** Ok, no payload, and the last reply. */
  end(): void { this.send(null, true); }

  /** An error, with a description for people; the last reply. */
  fail(errno: Errno, description: string = ""): void {
    if (errno == Errno.Ok) abort("fail needs an error, not Ok");
    this.finish();
    const d = String.UTF8.encode(description);
    host_fail(this.id, errno, changetype<usize>(d), d.byteLength);
  }

  /** Hands the request to `path` in our view; its answer goes straight to the caller. */
  forward(path: string): void {
    this.finish();
    const p = String.UTF8.encode(path);
    host_forward(this.id, changetype<usize>(p), p.byteLength);
  }

  private send(body: Payload | null, done: bool): void {
    if (done) this.finish();
    else if (this.finished) abort("request for " + this.path + " was already answered");
    const t = typeOf(body);
    const b = bytesOf(body);
    host_reply(this.id, changetype<usize>(t), t.byteLength, changetype<usize>(b), b.byteLength, done ? 1 : 0);
  }

  private finish(): void {
    if (this.finished) abort("request for " + this.path + " was already answered");
    this.finished = true;
    open.delete(this.id);
  }
}

/** Base class for objects. Override what you need. */
export class Server {
  /** Called once the image is set up: views are built, startup requests delivered. */
  start(): void {}

  /** A request made of our served tree. */
  serve(req: Request): void { req.fail(Errno.NotSupported); }

  /** The caller gave up on `req` (a stream, usually). It is ended for us afterwards. */
  cancel(req: Request): void {}
}

let server: Server | null = null;

/** Registers the module's object. Call once, at top level. */
export function run(s: Server): void {
  server = s;
}

function bound(): Server {
  const s = server;
  if (s === null) abort("no server; call run(new YourServer()) at top level");
  return s!;
}

// ---------------------------------------------------------------------------
// Calling

/** An answer to one of our calls. */
export class Response {
  constructor(
    public status: Errno,
    public body: Payload | null,
    /** False while a stream is still open. */
    public done: bool,
    /** An error's description, for people; "" if it has none. */
    public description: string = "",
  ) {}

  get ok(): bool { return this.status == Errno.Ok; }
}

/**
 * What to do with the answers to a call. AssemblyScript closures can't
 * capture variables, so subclass this and keep state in its fields.
 */
export class Then {
  answer(res: Response): void {}
}

/** Calls awaiting answers, by ticket. */
const calls = new Map<u32, Then>();

/** A call we made. */
export class Call {
  constructor(public ticket: u32) {}

  /** Gives up on the call; nothing more arrives for it. */
  cancel(): void {
    if (this.ticket == 0 || !calls.has(this.ticket)) return;
    calls.delete(this.ticket);
    host_cancel(this.ticket);
  }
}

export namespace ns {
  /** Makes a request of `path` in our view. With `then` null, answers are dropped. */
  export function call(verb: Verb, path: string, body: Payload | null, then: Then | null): Call {
    const p = String.UTF8.encode(path);
    const t = typeOf(body);
    const b = bytesOf(body);
    const ticket = host_call(
      verb, changetype<usize>(p), p.byteLength, changetype<usize>(t), t.byteLength,
      changetype<usize>(b), b.byteLength, then !== null ? 1 : 0,
    );
    if (then !== null) calls.set(ticket, then);
    return new Call(ticket);
  }

  export function read(path: string, then: Then): Call { return call(Verb.Read, path, null, then); }
  export function stat(path: string, then: Then): Call { return call(Verb.Stat, path, null, then); }
  export function list(path: string, then: Then): Call { return call(Verb.List, path, null, then); }
  export function watch(path: string, then: Then): Call { return call(Verb.Watch, path, null, then); }
  export function write(path: string, body: Payload | null, then: Then | null = null): Call { return call(Verb.Write, path, body, then); }
  export function create(path: string, body: Payload | null, then: Then | null = null): Call { return call(Verb.Create, path, body, then); }
  export function remove(path: string, then: Then | null = null): Call { return call(Verb.Remove, path, null, then); }

  /** Makes what `source` names in our view also appear at `at`. */
  export function bind(source: string, at: string, flags: Flag = Flag.None): Errno {
    const s = String.UTF8.encode(source);
    const a = String.UTF8.encode(at);
    return <Errno>host_bind(changetype<usize>(s), s.byteLength, changetype<usize>(a), a.byteLength, flags);
  }

  export function unmount(at: string): Errno {
    const a = String.UTF8.encode(at);
    return <Errno>host_unmount(changetype<usize>(a), a.byteLength);
  }

  /** Splices `source` (in our view) into our served tree at `at`; requests there skip our serve(). */
  export function expose(at: string, source: string, flags: Flag = Flag.None): Errno {
    const a = String.UTF8.encode(at);
    const s = String.UTF8.encode(source);
    return <Errno>host_expose(changetype<usize>(a), a.byteLength, changetype<usize>(s), s.byteLength, flags);
  }

  /** `root` followed by `path`, or null unless `path` is absolute with no `..` segment. */
  export function join(root: string, path: string): string | null {
    if (!path.startsWith("/")) return null;
    const segs = path.split("/");
    for (let i = 0; i < segs.length; i++) {
      if (segs[i] == "..") return null;
    }
    const base = root.endsWith("/") ? root.substring(0, root.length - 1) : root;
    return base + path;
  }
}

function typeOf(body: Payload | null): ArrayBuffer {
  return body === null ? new ArrayBuffer(0) : String.UTF8.encode(body.type);
}

function bytesOf(body: Payload | null): ArrayBuffer {
  return body === null ? new ArrayBuffer(0) : body.bytes;
}

/** The payload at `at` in the inbox: its type, then its bytes. */
function inboxPayload(at: i32, typeLen: i32, bodyLen: i32): Payload | null {
  if (typeLen == 0 && bodyLen == 0) return null;
  const type = String.UTF8.decodeUnsafe(changetype<usize>(inbox) + <usize>at, <usize>typeLen);
  return new Payload(type, inbox.slice(at + typeLen, at + typeLen + bodyLen));
}

// ---------------------------------------------------------------------------
// Host entry points. Object modules re-export these:
//   export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

let inbox = new ArrayBuffer(0);

/** Allocates a buffer the host writes the next path and payload into. */
export function rt_inbox(len: i32): usize {
  inbox = new ArrayBuffer(len);
  return changetype<usize>(inbox);
}

/** A request for us: the inbox holds the path, the payload's type, then its bytes. */
export function rt_serve(id: u32, verb: i32, flags: i32, pathLen: i32, typeLen: i32, bodyLen: i32): void {
  const path = String.UTF8.decodeUnsafe(changetype<usize>(inbox), <usize>pathLen);
  const req = new Request(id, <Verb>verb, path, inboxPayload(pathLen, typeLen, bodyLen), (flags & OPERATOR) != 0);
  open.set(id, req);
  bound().serve(req);
}

/**
 * An answer to one of our calls. For Ok, the inbox holds the payload's type,
 * then its bytes; for an error, its description.
 */
export function rt_answer(ticket: u32, errno: i32, typeLen: i32, bodyLen: i32, done: i32): void {
  if (!calls.has(ticket)) return;
  const then = calls.get(ticket);
  if (done != 0) calls.delete(ticket);
  if (errno != Errno.Ok) {
    const description = String.UTF8.decodeUnsafe(changetype<usize>(inbox), <usize>bodyLen);
    then.answer(new Response(<Errno>errno, null, true, description));
    return;
  }
  then.answer(new Response(Errno.Ok, inboxPayload(0, typeLen, bodyLen), done != 0));
}

export function rt_start(): void {
  bound().start();
}

export function rt_cancel(id: u32): void {
  if (!open.has(id)) return;
  const req = open.get(id);
  req.cancelled = true;
  bound().cancel(req);
  if (!req.finished) req.end();
}
