// Payloads: what a request or an answer carries besides its verb, path and
// status. A content type and some bytes, which the runtime carries without
// looking inside. The one layout the protocol defines itself is the stat
// record (see src/payload.rs), which `stat` and `list` answer with.

export const TEXT = "text/plain"
export const OCTETS = "application/octet-stream"
export const STAT = "application/x-othismo-stat"

/** A content type and some bytes. */
export class Payload {
  constructor(
    /** Its content type, e.g. "text/plain". */
    public type: string,
    public bytes: ArrayBuffer,
  ) {}

  /** `s` as UTF-8, `text/plain` unless another text type is given. */
  static text(s: string, type: string = TEXT): Payload {
    return new Payload(type, String.UTF8.encode(s))
  }

  /** Whether it is text of any kind: `text/*`. */
  isText(): bool {
    return this.type.toLowerCase().startsWith("text/")
  }

  /** The bytes, read as UTF-8. */
  asText(): string {
    return String.UTF8.decode(this.bytes)
  }
}

/** One entry in a served tree, as `stat` and `list` answer. */
export class Stat {
  constructor(
    /** The last segment of its path. */
    public name: string,
    public dir: bool,
    /** In bytes, or 0 for a directory or if it isn't known. */
    public length: u64 = 0,
    /** The content type a read would answer with, or "". */
    public type: string = "",
    /** Changes whenever what a read would answer changes, or 0. */
    public version: u64 = 0,
  ) {}

  static file(name: string, length: u64 = 0, type: string = ""): Stat {
    return new Stat(name, false, length, type)
  }

  static directory(name: string): Stat {
    return new Stat(name, true)
  }

  /** `name[s] kind[1] length[8] type[s] version[8]`, little-endian. */
  payload(): Payload {
    const name = String.UTF8.encode(this.name)
    const type = String.UTF8.encode(this.type)
    const out = new ArrayBuffer(2 + name.byteLength + 1 + 8 + 2 + type.byteLength + 8)
    let p = changetype<usize>(out)
    store<u16>(p, <u16>name.byteLength)
    memory.copy(p + 2, changetype<usize>(name), <usize>name.byteLength)
    p += 2 + <usize>name.byteLength
    store<u8>(p, this.dir ? 1 : 0)
    store<u64>(p + 1, this.length)
    p += 9
    store<u16>(p, <u16>type.byteLength)
    memory.copy(p + 2, changetype<usize>(type), <usize>type.byteLength)
    p += 2 + <usize>type.byteLength
    store<u64>(p, this.version)
    return new Payload(STAT, out)
  }

  /** The stat record in `p`, or null if it isn't a well-formed one. */
  static decode(p: Payload | null): Stat | null {
    if (p === null || p.type != STAT) return null
    const b = p.bytes
    const base = changetype<usize>(b)
    const end = <usize>b.byteLength
    let at: usize = 0
    if (at + 2 > end) return null
    const nameLen = <usize>load<u16>(base + at)
    at += 2
    if (at + nameLen + 9 + 2 > end) return null
    const name = String.UTF8.decodeUnsafe(base + at, nameLen)
    at += nameLen
    const dir = load<u8>(base + at) != 0
    const length = load<u64>(base + at + 1)
    at += 9
    const typeLen = <usize>load<u16>(base + at)
    at += 2
    if (at + typeLen + 8 != end) return null
    const type = String.UTF8.decodeUnsafe(base + at, typeLen)
    at += typeLen
    return new Stat(name, dir, length, type, load<u64>(base + at))
  }
}

/**
 * Whether a payload of content type `given` fits where `want` is written:
 * `*\/*` takes anything and `text/*` any text, and a parameter `want` writes
 * must be in `given` with the same value (see src/payload.rs).
 */
export function fits(want: string, given: string): bool {
  const w = want.split(";")
  const g = given.split(";")
  const wt = w[0].trim().toLowerCase()
  const gt = g[0].trim().toLowerCase()
  const ws = wt.indexOf("/")
  const gs = gt.indexOf("/")
  if (ws < 0 || gs < 0) return false
  const wType = wt.substring(0, ws), wSub = wt.substring(ws + 1)
  const gType = gt.substring(0, gs), gSub = gt.substring(gs + 1)
  if (wType != "*" && (wType != gType || (wSub != "*" && wSub != gSub))) return false
  for (let i = 1; i < w.length; i++) {
    let found = false
    for (let j = 1; j < g.length; j++) {
      if (w[i].trim() == g[j].trim()) found = true
    }
    if (!found) return false
  }
  return true
}
