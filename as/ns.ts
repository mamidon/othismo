// The namespace SDK: how an object serves its tree and talks to its view.
//
// An object module extends `Server`, calls `run(new MyServer())` at top
// level, and re-exports the host's entry points:
//
//   export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }
//
// Requests are made with the `ns` functions and answered through `Then`
// objects; requests made of us arrive at `Server.serve`. See
// docs/namespace.md for the model.

import { Message, encodeMessage, decodeMessage } from "./index"

// ---------------------------------------------------------------------------
// Host interface

// @ts-ignore: decorator
@external("othismo", "call")
declare function host_call(verb: i32, path: usize, pathLen: i32, body: usize, bodyLen: i32, expect: i32): u32;

// @ts-ignore: decorator
@external("othismo", "reply")
declare function host_reply(req: u32, status: i32, body: usize, bodyLen: i32, done: i32): void;

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
// Verbs, statuses, flags (numbered as in src/ns.rs)

/** Control is the operator's verb: the image may send it, objects can't. */
export enum Verb { Read, Stat, List, Watch, Write, Create, Remove, Invoke, Control }

/** Safe verbs never change anything; read-only mounts let only these through. */
export function isSafe(v: Verb): bool {
  return v < Verb.Write;
}

export enum Status { Ok, NotFound, ReadOnly, NotSupported, IsDirectory, NotDirectory, Exists, Invalid, Io, Forbidden }

export enum Flag { None = 0, ReadOnly = 1 }

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
    public body: Message | null,
  ) {}

  /** Ok, and the last reply. */
  reply(body: Message | null): void { this.send(Status.Ok, body, true); }

  /** Ok, with more to come. */
  stream(body: Message): void { this.send(Status.Ok, body, false); }

  /** Ok, no body, and the last reply. */
  end(): void { this.send(Status.Ok, null, true); }

  /** A failure; the last reply. */
  fail(status: Status): void { this.send(status, null, true); }

  /** Hands the request to `path` in our view; its answer goes straight to the caller. */
  forward(path: string): void {
    this.finish();
    const p = String.UTF8.encode(path);
    host_forward(this.id, changetype<usize>(p), p.byteLength);
  }

  private send(status: Status, body: Message | null, done: bool): void {
    if (done) this.finish();
    else if (this.finished) abort("request for " + this.path + " was already answered");
    let b = new ArrayBuffer(0);
    if (body !== null) b = encodeMessage(body);
    host_reply(this.id, status, changetype<usize>(b), b.byteLength, done ? 1 : 0);
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
  serve(req: Request): void { req.fail(Status.NotSupported); }

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
    public status: Status,
    public body: Message | null,
    /** False while a stream is still open. */
    public done: bool,
  ) {}

  get ok(): bool { return this.status == Status.Ok; }
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
  export function call(verb: Verb, path: string, body: Message | null, then: Then | null): Call {
    const p = String.UTF8.encode(path);
    let b = new ArrayBuffer(0);
    if (body !== null) b = encodeMessage(body);
    const ticket = host_call(verb, changetype<usize>(p), p.byteLength, changetype<usize>(b), b.byteLength, then !== null ? 1 : 0);
    if (then !== null) calls.set(ticket, then);
    return new Call(ticket);
  }

  export function read(path: string, then: Then): Call { return call(Verb.Read, path, null, then); }
  export function stat(path: string, then: Then): Call { return call(Verb.Stat, path, null, then); }
  export function list(path: string, then: Then): Call { return call(Verb.List, path, null, then); }
  export function watch(path: string, then: Then): Call { return call(Verb.Watch, path, null, then); }
  export function write(path: string, body: Message, then: Then | null = null): Call { return call(Verb.Write, path, body, then); }
  export function create(path: string, body: Message, then: Then | null = null): Call { return call(Verb.Create, path, body, then); }
  export function remove(path: string, then: Then | null = null): Call { return call(Verb.Remove, path, null, then); }
  export function invoke(path: string, body: Message, then: Then | null = null): Call { return call(Verb.Invoke, path, body, then); }

  /** Makes what `source` names in our view also appear at `at`. */
  export function bind(source: string, at: string, flags: Flag = Flag.None): Status {
    const s = String.UTF8.encode(source);
    const a = String.UTF8.encode(at);
    return <Status>host_bind(changetype<usize>(s), s.byteLength, changetype<usize>(a), a.byteLength, flags);
  }

  export function unmount(at: string): Status {
    const a = String.UTF8.encode(at);
    return <Status>host_unmount(changetype<usize>(a), a.byteLength);
  }

  /** Splices `source` (in our view) into our served tree at `at`; requests there skip our serve(). */
  export function expose(at: string, source: string, flags: Flag = Flag.None): Status {
    const a = String.UTF8.encode(at);
    const s = String.UTF8.encode(source);
    return <Status>host_expose(changetype<usize>(a), a.byteLength, changetype<usize>(s), s.byteLength, flags);
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

// ---------------------------------------------------------------------------
// Host entry points. Object modules re-export these:
//   export { rt_inbox, rt_serve, rt_answer, rt_start, rt_cancel }

let inbox = new ArrayBuffer(0);

/** Allocates a buffer the host writes the next path and body into. */
export function rt_inbox(len: i32): usize {
  inbox = new ArrayBuffer(len);
  return changetype<usize>(inbox);
}

/** A request for us: the inbox holds the path, then the body. */
export function rt_serve(id: u32, verb: i32, pathLen: i32, bodyLen: i32): void {
  const path = String.UTF8.decodeUnsafe(changetype<usize>(inbox), <usize>pathLen);
  let body: Message | null = null;
  if (bodyLen > 0) body = decodeMessage(inbox.slice(pathLen, pathLen + bodyLen));
  const req = new Request(id, <Verb>verb, path, body);
  open.set(id, req);
  bound().serve(req);
}

/** An answer to one of our calls: the inbox holds the body. */
export function rt_answer(ticket: u32, status: i32, bodyLen: i32, done: i32): void {
  if (!calls.has(ticket)) return;
  const then = calls.get(ticket);
  if (done != 0) calls.delete(ticket);
  let body: Message | null = null;
  if (bodyLen > 0) body = decodeMessage(inbox);
  then.answer(new Response(<Status>status, body, done != 0));
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
