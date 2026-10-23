//! Instantiates a built [`Pack`] and runs its message loop.
//!
//! Every message is a request to a path or a reply to one. A request is
//! resolved in the sender's view (see [`crate::ns`]) when it is made, and
//! queued for the instance that serves it. Replies go back to whoever made
//! the request, tagged with the ticket the caller was given.
//!
//! Delivery is synchronous and single-threaded: one FIFO queue, drained
//! completely before the next outside [`Event`] is taken.
//!
//! Unless checks are off, every object is held to its interface on the way:
//! what it asks must be in its `needs`, what reaches it must be in its
//! `serves`, and only a stream may be answered more than once. A request
//! that doesn't fit is answered with a status and never arrives; an answer
//! that doesn't fit is its sender's fault and stops the run. Payloads are
//! carried as they are: the runtime never looks inside one.
//!
//! Wasm objects get a [`Store`] each and an import set of exactly `env.abort`
//! and the `othismo` functions in [`ALLOWED_IMPORTS`]; anything else a
//! module asks for is refused before it is instantiated.

use crate::check::{Kind, MountDecl, Program};
use crate::pack::Pack;
use crate::env::{self, EnvObject, Event, Reply};
use crate::iface;
use crate::ns::{self, Namespace, Resolved, Status, Verb};
use crate::payload::Payload;
use anyhow::{Context, Result, anyhow, bail};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::rc::Rc;
use std::sync::mpsc;
use wasmtime::{Caller, Engine, Extern, Linker, Memory, Module, Store, TypedFunc};

/// The outside world's stdin/stdout, as seen by the environmental objects.
/// Tests swap in in-memory buffers; the image doesn't change.
pub struct Io {
    pub input: Box<dyn BufRead + Send>,
    pub output: Box<dyn Write>,
}

impl Io {
    pub fn std() -> Io {
        Io { input: Box::new(std::io::BufReader::new(std::io::stdin())), output: Box::new(std::io::stdout()) }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Options {
    /// Don't hold objects to their interfaces.
    pub no_checks: bool,
}

/// Every host function a module may import.
pub const ALLOWED_IMPORTS: &[(&str, &str)] = &[
    ("env", "abort"),
    ("othismo", "call"),
    ("othismo", "reply"),
    ("othismo", "fail"),
    ("othismo", "forward"),
    ("othismo", "cancel"),
    ("othismo", "bind"),
    ("othismo", "unmount"),
    ("othismo", "expose"),
];

/// A reply to a request made with [`Runtime::ask`].
#[derive(Debug, Clone, PartialEq)]
pub struct Answered {
    pub status: Status,
    pub body: Option<Payload>,
    /// An error's description, for people; empty if it has none.
    pub detail: String,
    /// The last reply.
    pub done: bool,
}

/// Who made a request, and so where its replies go.
#[derive(Debug, Clone)]
enum Requester {
    /// A startup request, described for warnings. Replies are dropped.
    Image(String),
    /// A request made with [`Runtime::ask`]. Replies are kept for it.
    Asked,
    /// Ticket 0 means no answer is wanted.
    Wasm { inst: usize, ticket: u32 },
}

impl Requester {
    /// Whether whoever runs the image made the request: an object never
    /// does, and a request it forwards keeps its first sender's flag.
    fn is_operator(&self) -> bool {
        !matches!(self, Requester::Wasm { .. })
    }
}

/// A request being served.
struct Pending {
    caller: Requester,
    verb: Verb,
    /// Kept so the request can be forwarded.
    body: Option<Payload>,
    /// The caller gave up; replies are dropped.
    cancelled: bool,
    /// What the server's interface promises to answer, unless checks are off.
    promise: Option<Promise>,
}

struct Promise {
    /// The request, for errors: `read /hits`.
    what: String,
    /// Whether it may be answered more than once.
    stream: bool,
}

enum Delivery {
    Serve { to: usize, req: u32, verb: Verb, operator: bool, path: String, body: Option<Payload> },
    Answer { to: usize, ticket: u32, status: Status, body: Option<Payload>, detail: String, done: bool },
    Cancel { to: usize, req: u32 },
}

/// The state host functions work on. Shared between the runtime and every
/// Wasm store, and only ever borrowed briefly, never across a call into Wasm.
struct Shared {
    names: Vec<String>,
    ns: Namespace,
    /// Each instance's interface.
    interfaces: Vec<iface::Object>,
    checks: bool,
    queue: VecDeque<Delivery>,
    /// Replies to [`Requester::Asked`], until [`Runtime::ask`] takes them.
    asked: Vec<Answered>,
    /// `(server, req)`.
    requests: HashMap<(usize, u32), Pending>,
    next_req: Vec<u32>,
    /// `(caller, ticket) -> (server, req)`, for cancelling.
    calls: HashMap<(usize, u32), (usize, u32)>,
    next_ticket: Vec<u32>,
}

impl Shared {
    /// Where `path` in `inst`'s view leads, for a request `inst` makes:
    /// nowhere if a read-only mount is in the way of an unsafe verb, or if
    /// the request isn't one of `inst`'s needs.
    fn resolve(&self, inst: usize, path: &str, verb: Verb) -> Result<Resolved, Status> {
        let r = self.ns.resolve(inst, path)?;
        if r.read_only && !verb.is_safe() {
            return Err(Status::ReadOnly);
        }
        if self.checks {
            let (at, rest) = self.ns.mount_of(inst, path).ok_or(Status::NotFound)?;
            let need = self.interfaces[inst].need(&at).ok_or(Status::NotFound)?;
            need.entry(&rest).ok_or(Status::NotFound)?.takes_verb(verb, false)?;
        }
        Ok(r)
    }

    /// Makes a request of wherever `to` resolved; failures to resolve, and
    /// requests the server's interface doesn't take, are answered straight away.
    fn request(&mut self, caller: Requester, to: Result<Resolved, Status>, verb: Verb, body: Option<Payload>) {
        let r = match to {
            Ok(r) => r,
            Err(status) => return self.answer(&caller, status, None, String::new(), true),
        };
        let operator = caller.is_operator();
        let entry = self.interfaces[r.inst].serving(&r.path);
        // Like a read-only mount, the operator's requests are off limits to
        // objects whether or not checks are on.
        if !operator && entry.is_some_and(|e| e.takes_verb(verb, false) == Err(Status::Forbidden)) {
            return self.answer(&caller, Status::Forbidden, None, String::new(), true);
        }
        let promise = if self.checks {
            match entry.ok_or(Status::NotFound).and_then(|e| e.takes_verb(verb, operator).map(|_| e.streams(verb))) {
                Ok(stream) => Some(Promise { what: format!("{verb} {}", r.path), stream }),
                Err(status) => return self.answer(&caller, status, None, String::new(), true),
            }
        } else {
            None
        };
        self.next_req[r.inst] += 1;
        let req = self.next_req[r.inst];
        if let Requester::Wasm { inst, ticket } = caller
            && ticket != 0
        {
            self.calls.insert((inst, ticket), (r.inst, req));
        }
        self.requests.insert((r.inst, req), Pending { caller, verb, body: body.clone(), cancelled: false, promise });
        self.queue.push_back(Delivery::Serve { to: r.inst, req, verb, operator, path: r.path, body });
    }

    fn answer(&mut self, caller: &Requester, status: Status, body: Option<Payload>, detail: String, done: bool) {
        match *caller {
            Requester::Image(ref what) => {
                if status != Status::Ok {
                    let detail = if detail.is_empty() { String::new() } else { format!(": {detail}") };
                    eprintln!("warning: startup request `{what}` failed: {status}{detail}");
                }
            }
            Requester::Asked => self.asked.push(Answered { status, body, detail, done }),
            Requester::Wasm { ticket: 0, .. } => {}
            Requester::Wasm { inst, ticket } => {
                if done {
                    self.calls.remove(&(inst, ticket));
                }
                self.queue.push_back(Delivery::Answer { to: inst, ticket, status, body, detail, done });
            }
        }
    }

    /// `server` replies to its request `req`. An error is always the last
    /// reply, and may carry a description but no payload.
    fn reply(&mut self, server: usize, req: u32, status: Status, body: Option<Payload>, detail: String, done: bool) -> Result<()> {
        let Some(p) = self.requests.get(&(server, req)) else {
            bail!("instance `{}` replied to unknown request {req}", self.names[server]);
        };
        if let Some(promise) = &p.promise
            && !done
            && !promise.stream
        {
            let what = &promise.what;
            bail!("instance `{}` answered `{what}` more than once, but its interface promises one answer", self.names[server]);
        }
        let (caller, cancelled) = (p.caller.clone(), p.cancelled);
        if done {
            self.requests.remove(&(server, req));
        }
        if !cancelled {
            self.answer(&caller, status, body, detail, done);
        }
        Ok(())
    }

    /// `server` hands its request `req` to `path` in its own view.
    fn forward(&mut self, server: usize, req: u32, path: &str) -> Result<()> {
        let Some(p) = self.requests.remove(&(server, req)) else {
            bail!("instance `{}` forwarded unknown request {req}", self.names[server]);
        };
        if p.cancelled {
            return Ok(());
        }
        let to = self.resolve(server, path, p.verb);
        self.request(p.caller, to, p.verb, p.body);
        Ok(())
    }

    /// `caller` gives up on the call it was given `ticket` for.
    fn cancel(&mut self, caller: usize, ticket: u32) {
        let Some((server, req)) = self.calls.remove(&(caller, ticket)) else { return };
        if let Some(p) = self.requests.get_mut(&(server, req)) {
            p.cancelled = true;
            self.queue.push_back(Delivery::Cancel { to: server, req });
        }
    }
}

struct HostState {
    me: usize,
    shared: Rc<RefCell<Shared>>,
}

struct WasmObject {
    store: Store<HostState>,
    memory: Memory,
    inbox: TypedFunc<i32, i32>,
    serve: TypedFunc<(i32, i32, i32, i32, i32, i32), ()>,
    answer: TypedFunc<(i32, i32, i32, i32, i32), ()>,
    start: TypedFunc<(), ()>,
    /// Optional: a module that never streams needn't hear about cancels.
    /// Without it, a cancelled request's replies are dropped until it ends.
    cancel: Option<TypedFunc<i32, ()>>,
}

impl WasmObject {
    /// Copies `bytes` into a fresh inbox in the module's memory.
    fn fill_inbox(&mut self, bytes: &[u8]) -> Result<()> {
        let ptr = self.inbox.call(&mut self.store, bytes.len() as i32)?;
        self.memory.write(&mut self.store, ptr as u32 as usize, bytes)?;
        Ok(())
    }
}

enum Object {
    Wasm(Box<WasmObject>),
    Env(EnvObject),
    /// An instance of an image, which serves only through what it exports:
    /// a request that lands on it found nothing there.
    Image,
}

pub struct Runtime<'p> {
    program: &'p Program,
    objects: Vec<Object>,
    shared: Rc<RefCell<Shared>>,
    input: Option<Box<dyn BufRead + Send>>,
    output: Box<dyn Write>,
}

/// Runs `pack` until no outside event can arrive any more (stdin is
/// exhausted and nothing is listening) and every queued message is delivered.
pub fn run(pack: &Pack, io: Io, options: Options) -> Result<()> {
    Runtime::new(pack, io, options)?.run()
}

impl<'p> Runtime<'p> {
    /// Builds every view and instantiates every object. Requests made by
    /// module top-level code are queued, not yet delivered.
    pub fn new(pack: &'p Pack, io: Io, options: Options) -> Result<Runtime<'p>> {
        let program = &pack.program;
        let n = program.instances.len();
        // One view per instance, and the root image's own after them. An
        // instance of an image serves only what it exports.
        let mut ns = Namespace::new(n + 1);
        let resolved = |m: &MountDecl| Resolved { inst: m.target, path: m.path.clone(), read_only: m.read_only };
        for (i, inst) in program.instances.iter().enumerate() {
            for m in &inst.mounts {
                ns.mount_resolved(i, &m.at, resolved(m));
            }
            if let Kind::Image { exports } = &inst.kind {
                for e in exports {
                    ns.expose_resolved(i, &e.at, resolved(e));
                }
            }
        }
        for m in &program.view {
            ns.mount_resolved(n, &m.at, resolved(m));
        }
        let interfaces = program
            .instances
            .iter()
            .map(|i| program.interfaces.get(&i.type_name).cloned().ok_or_else(|| anyhow!("instance `{}` has no interface", i.name)))
            .collect::<Result<_>>()?;
        let shared = Rc::new(RefCell::new(Shared {
            names: program.instances.iter().map(|i| i.name.clone()).collect(),
            ns,
            interfaces,
            checks: !options.no_checks,
            queue: VecDeque::new(),
            asked: vec![],
            requests: HashMap::new(),
            next_req: vec![0; n],
            calls: HashMap::new(),
            next_ticket: vec![0; n],
        }));

        let engine = Engine::default();
        let linker = host_linker(&engine)?;
        let mut objects = vec![];
        for (i, inst) in program.instances.iter().enumerate() {
            let obj = match &inst.kind {
                Kind::Wasm { type_name, .. } => {
                    let state = HostState { me: i, shared: shared.clone() };
                    let module = pack.blob(&pack.modules[type_name]);
                    Object::Wasm(Box::new(instantiate(&engine, &linker, &inst.name, type_name, module, state)?))
                }
                Kind::Image { .. } => Object::Image,
                kind => Object::Env(
                    EnvObject::new(inst, pack)?
                        .ok_or_else(|| anyhow!("no environment object for {kind:?}"))
                        .with_context(|| format!("instance `{}`", inst.name))?,
                ),
            };
            objects.push(obj);
        }
        Ok(Runtime { program, objects, shared, input: Some(io.input), output: io.output })
    }

    /// Runs the image: starts it, then the outside world, until nothing
    /// more can arrive.
    pub fn run(mut self) -> Result<()> {
        self.start()?;
        self.outside()
    }

    /// Delivers the image's startup requests and runs every module's
    /// `start`, but starts nothing outside: no stdin, no sockets.
    pub fn start(&mut self) -> Result<()> {
        // Every image's startup requests, in the order they are made, each
        // resolved in its image's view.
        let root_view = self.program.instances.len();
        for s in &self.program.startup {
            let what = match s.image {
                Some(i) => format!("{} {} in {}", s.verb.keyword(), s.path, self.program.instances[i].name),
                None => format!("{} {}", s.verb.keyword(), s.path),
            };
            let body = s.body.as_ref().map(|b| b.payload()).transpose()?;
            let mut shared = self.shared.borrow_mut();
            let to = shared.ns.resolve(s.image.unwrap_or(root_view), &s.path).and_then(|r| {
                if r.read_only && !s.verb.is_safe() { Err(Status::ReadOnly) } else { Ok(r) }
            });
            shared.request(Requester::Image(what), to, s.verb, body);
        }
        self.drain()?;
        // Then every module's start(), each instance after everything it
        // mounts, and an image's after everything it exports.
        let mounts: Vec<Vec<usize>> = self
            .program
            .instances
            .iter()
            .map(|i| {
                let exports = match &i.kind {
                    Kind::Image { exports } => exports.as_slice(),
                    _ => &[],
                };
                i.mounts.iter().chain(exports).map(|m| m.target).collect()
            })
            .collect();
        let order = ns::start_order(&mounts);
        for &i in &order {
            if let Object::Wasm(obj) = &mut self.objects[i] {
                obj.start.call(&mut obj.store, ()).map_err(|e| self.failed(i, "starting", e))?;
            }
        }
        self.drain()
    }

    /// As the operator, makes `verb <path>` of instance `inst`'s served tree,
    /// delivers everything that follows from it, and returns the replies it
    /// has had: none at all if nothing answered.
    pub fn ask(&mut self, inst: usize, verb: Verb, path: &str, body: Option<Payload>) -> Result<Vec<Answered>> {
        let to = Ok(Resolved { inst, path: path.to_string(), read_only: false });
        self.shared.borrow_mut().request(Requester::Asked, to, verb, body);
        self.drain()?;
        Ok(std::mem::take(&mut self.shared.borrow_mut().asked))
    }

    /// Starts the outside world, and delivers what it sends until nothing
    /// more can arrive.
    fn outside(mut self) -> Result<()> {

        // Then the outside world. Stdin is read only if something watches it.
        let (tx, rx) = mpsc::channel::<Event>();
        let consoles: Vec<usize> = (0..self.objects.len())
            .filter(|&i| matches!(&self.objects[i], Object::Env(EnvObject::Console(c)) if c.is_watched()))
            .collect();
        if !consoles.is_empty() {
            env::spawn_stdin(self.input.take().expect("input is taken once"), consoles, tx.clone());
        }
        for (i, obj) in self.objects.iter_mut().enumerate() {
            if let Object::Env(e) = obj {
                e.start(i, &tx);
            }
        }
        // Once every source thread has finished, `rx` disconnects and we're done.
        drop(tx);
        for ev in rx {
            let Object::Env(e) = &mut self.objects[ev.to] else { unreachable!("events only come from env objects") };
            let replies = e.on_event(ev.what);
            self.send_replies(ev.to, replies)?;
            self.drain()?;
        }
        Ok(())
    }

    fn failed(&self, inst: usize, doing: &str, e: impl std::fmt::Display) -> anyhow::Error {
        anyhow!("instance `{}` failed {doing}: {e:#}", self.program.instances[inst].name)
    }

    fn drain(&mut self) -> Result<()> {
        loop {
            let next = self.shared.borrow_mut().queue.pop_front();
            match next {
                Some(d) => self.deliver(d)?,
                None => return Ok(()),
            }
        }
    }

    fn deliver(&mut self, d: Delivery) -> Result<()> {
        match d {
            Delivery::Serve { to, req, verb, operator, path, body } => {
                let doing = format!("serving `{} {path}`", verb.keyword());
                match &mut self.objects[to] {
                    Object::Env(e) => {
                        let replies =
                            e.serve(req, verb, &path, body.as_ref(), &mut self.output).map_err(|e| self.failed(to, &doing, e))?;
                        self.send_replies(to, replies)?;
                    }
                    Object::Image => {
                        self.shared.borrow_mut().reply(to, req, Status::NotFound, None, String::new(), true)?;
                    }
                    Object::Wasm(obj) => {
                        let (ty, body) = body.map(|b| (b.ty, b.bytes)).unwrap_or_default();
                        let mut bytes = path.as_bytes().to_vec();
                        bytes.extend_from_slice(ty.as_bytes());
                        bytes.extend_from_slice(&body);
                        let flags = if operator { ns::OPERATOR } else { 0 };
                        let args = (req as i32, verb as i32, flags, path.len() as i32, ty.len() as i32, body.len() as i32);
                        let res = obj.fill_inbox(&bytes).and_then(|_| Ok(obj.serve.call(&mut obj.store, args)?));
                        res.map_err(|e| self.failed(to, &doing, e))?;
                    }
                }
            }
            Delivery::Answer { to, ticket, status, body, detail, done } => {
                let Object::Wasm(obj) = &mut self.objects[to] else { unreachable!("only Wasm objects make calls") };
                // An error's description takes the place of a payload.
                let (ty, body) = match body {
                    Some(b) => (b.ty, b.bytes),
                    None => (String::new(), detail.into_bytes()),
                };
                let mut bytes = ty.as_bytes().to_vec();
                bytes.extend_from_slice(&body);
                let args = (ticket as i32, status as i32, ty.len() as i32, body.len() as i32, done as i32);
                let res = obj.fill_inbox(&bytes).and_then(|_| Ok(obj.answer.call(&mut obj.store, args)?));
                res.map_err(|e| self.failed(to, "handling an answer", e))?;
            }
            Delivery::Cancel { to, req } => match &mut self.objects[to] {
                Object::Image => {}
                Object::Env(e) => {
                    let replies = e.cancel(req);
                    self.send_replies(to, replies)?;
                }
                // Without rt_cancel, its replies are dropped until it ends the request on its own.
                Object::Wasm(obj) => {
                    if let Some(f) = obj.cancel.clone() {
                        f.call(&mut obj.store, req as i32).map_err(|e| self.failed(to, "cancelling", e))?;
                    }
                }
            },
        }
        Ok(())
    }

    fn send_replies(&mut self, from: usize, replies: Vec<Reply>) -> Result<()> {
        let mut shared = self.shared.borrow_mut();
        for r in replies {
            shared.reply(from, r.req, r.status, r.body, String::new(), r.done)?;
        }
        Ok(())
    }
}

fn instantiate(
    engine: &Engine,
    linker: &Linker<HostState>,
    name: &str,
    type_name: &str,
    bytes: &[u8],
    state: HostState,
) -> Result<WasmObject> {
    let module = Module::new(engine, bytes)
        .map_err(anyhow::Error::from)
        .with_context(|| format!("instance `{name}`: invalid module {type_name}"))?;
    for imp in module.imports() {
        if !ALLOWED_IMPORTS.contains(&(imp.module(), imp.name())) {
            bail!(
                "instance `{name}`: module {type_name} imports `{}.{}`, which the runtime does not provide",
                imp.module(),
                imp.name()
            );
        }
    }
    let mut store = Store::new(engine, state);
    let instance =
        linker.instantiate(&mut store, &module).map_err(|e| anyhow!("instance `{name}` failed to start: {e:#}"))?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow!("instance `{name}`: module does not export `memory`"))?;
    let mut export = |func: &str, sig: &str| {
        instance
            .get_func(&mut store, func)
            .ok_or_else(|| anyhow!("instance `{name}`: module must export {func}{sig}"))
    };
    let inbox = export("rt_inbox", "(i32) -> i32")?;
    let serve = export("rt_serve", "(i32, i32, i32, i32, i32, i32)")?;
    let answer = export("rt_answer", "(i32, i32, i32, i32, i32)")?;
    let start = export("rt_start", "()")?;
    let typed = |e: wasmtime::Error, func: &str| anyhow!("instance `{name}`: {func} has the wrong signature: {e}");
    Ok(WasmObject {
        inbox: inbox.typed(&store).map_err(|e| typed(e, "rt_inbox"))?,
        serve: serve.typed(&store).map_err(|e| typed(e, "rt_serve"))?,
        answer: answer.typed(&store).map_err(|e| typed(e, "rt_answer"))?,
        start: start.typed(&store).map_err(|e| typed(e, "rt_start"))?,
        cancel: match instance.get_func(&mut store, "rt_cancel") {
            Some(f) => Some(f.typed(&store).map_err(|e| typed(e, "rt_cancel"))?),
            None => None,
        },
        memory,
        store,
    })
}

type HostResult<T> = wasmtime::Result<T>;

fn host_err(e: anyhow::Error) -> wasmtime::Error {
    wasmtime::format_err!("{e:#}")
}

/// A path from module memory; invalid UTF-8 can't name anything.
fn read_path(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> HostResult<Option<String>> {
    Ok(String::from_utf8(read_bytes(caller, ptr, len)?).ok())
}

/// An optional payload from module memory: a content type and bytes, both
/// of length 0 for none. It is copied as it is, never looked inside.
fn read_body(caller: &mut Caller<'_, HostState>, ty: i32, ty_len: i32, body: i32, body_len: i32) -> HostResult<Option<Payload>> {
    if ty_len == 0 && body_len == 0 {
        return Ok(None);
    }
    let me = caller.data().me;
    let ty = String::from_utf8(read_bytes(caller, ty, ty_len)?);
    let bytes = read_bytes(caller, body, body_len)?;
    let name = caller.data().shared.borrow().names[me].clone();
    match ty {
        Ok(ty) if !ty.is_empty() => Ok(Some(Payload::new(ty, bytes))),
        _ => Err(wasmtime::format_err!("instance `{name}` sent a payload without a content type")),
    }
}

fn status_code(r: Result<(), Status>) -> i32 {
    match r {
        Ok(()) => Status::Ok as i32,
        Err(s) => s as i32,
    }
}

fn host_linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    linker.func_wrap(
        "othismo",
        "call",
        |mut caller: Caller<'_, HostState>,
         verb: i32,
         path: i32,
         path_len: i32,
         ty: i32,
         ty_len: i32,
         body: i32,
         body_len: i32,
         expect: i32| {
            let me = caller.data().me;
            let path = read_path(&mut caller, path, path_len)?;
            let body = read_body(&mut caller, ty, ty_len, body, body_len)?;
            let mut shared = caller.data().shared.borrow_mut();
            let verb = Verb::from_i32(verb).ok_or_else(|| wasmtime::format_err!("call with unknown verb {verb}"))?;
            let ticket = if expect != 0 {
                shared.next_ticket[me] += 1;
                shared.next_ticket[me]
            } else {
                0
            };
            let to = match path {
                None => Err(Status::Invalid),
                Some(p) => shared.resolve(me, &p, verb),
            };
            shared.request(Requester::Wasm { inst: me, ticket }, to, verb, body);
            HostResult::Ok(ticket as i32)
        },
    )?;
    linker.func_wrap(
        "othismo",
        "reply",
        |mut caller: Caller<'_, HostState>, req: i32, ty: i32, ty_len: i32, body: i32, body_len: i32, done: i32| -> HostResult<()> {
            let me = caller.data().me;
            let body = read_body(&mut caller, ty, ty_len, body, body_len)?;
            let mut shared = caller.data().shared.borrow_mut();
            shared.reply(me, req as u32, Status::Ok, body, String::new(), done != 0).map_err(host_err)
        },
    )?;
    linker.func_wrap(
        "othismo",
        "fail",
        |mut caller: Caller<'_, HostState>, req: i32, errno: i32, desc: i32, desc_len: i32| -> HostResult<()> {
            let me = caller.data().me;
            let detail = String::from_utf8_lossy(&read_bytes(&mut caller, desc, desc_len)?).into_owned();
            let status = Status::from_i32(errno).filter(|s| *s != Status::Ok);
            let status = status.ok_or_else(|| wasmtime::format_err!("fail with errno {errno}, which isn't an error"))?;
            let mut shared = caller.data().shared.borrow_mut();
            shared.reply(me, req as u32, status, None, detail, true).map_err(host_err)
        },
    )?;
    linker.func_wrap(
        "othismo",
        "forward",
        |mut caller: Caller<'_, HostState>, req: i32, path: i32, path_len: i32| -> HostResult<()> {
            let me = caller.data().me;
            // Invalid UTF-8 resolves as an invalid path.
            let path = read_path(&mut caller, path, path_len)?.unwrap_or_default();
            let mut shared = caller.data().shared.borrow_mut();
            shared.forward(me, req as u32, &path).map_err(host_err)
        },
    )?;
    linker.func_wrap("othismo", "cancel", |caller: Caller<'_, HostState>, ticket: i32| {
        let me = caller.data().me;
        caller.data().shared.borrow_mut().cancel(me, ticket as u32);
    })?;
    linker.func_wrap(
        "othismo",
        "bind",
        |mut caller: Caller<'_, HostState>, src: i32, src_len: i32, at: i32, at_len: i32, flags: i32| -> HostResult<i32> {
            let me = caller.data().me;
            let (src, at) = (read_path(&mut caller, src, src_len)?, read_path(&mut caller, at, at_len)?);
            let mut shared = caller.data().shared.borrow_mut();
            Ok(status_code(match (src, at) {
                (Some(src), Some(at)) => shared.ns.bind(me, &src, &at, flags & ns::READ_ONLY != 0),
                _ => Err(Status::Invalid),
            }))
        },
    )?;
    linker.func_wrap("othismo", "unmount", |mut caller: Caller<'_, HostState>, at: i32, at_len: i32| -> HostResult<i32> {
        let me = caller.data().me;
        let at = read_path(&mut caller, at, at_len)?;
        let mut shared = caller.data().shared.borrow_mut();
        Ok(status_code(at.ok_or(Status::Invalid).and_then(|at| shared.ns.unmount(me, &at))))
    })?;
    linker.func_wrap(
        "othismo",
        "expose",
        |mut caller: Caller<'_, HostState>, at: i32, at_len: i32, src: i32, src_len: i32, flags: i32| -> HostResult<i32> {
            let me = caller.data().me;
            let (at, src) = (read_path(&mut caller, at, at_len)?, read_path(&mut caller, src, src_len)?);
            let mut shared = caller.data().shared.borrow_mut();
            Ok(status_code(match (at, src) {
                (Some(at), Some(src)) => shared.ns.expose(me, &at, &src, flags & ns::READ_ONLY != 0),
                _ => Err(Status::Invalid),
            }))
        },
    )?;
    linker.func_wrap(
        "env",
        "abort",
        |mut caller: Caller<'_, HostState>, msg: i32, file: i32, line: i32, col: i32| -> HostResult<()> {
            let mem = memory(&mut caller)?;
            let msg = read_as_string(mem.data(&caller), msg);
            let file = read_as_string(mem.data(&caller), file);
            Err(wasmtime::format_err!("abort: {msg} ({file}:{line}:{col})"))
        },
    )?;
    Ok(linker)
}

fn read_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> HostResult<Vec<u8>> {
    let mem = memory(caller)?;
    let mut buf = vec![0; len as u32 as usize];
    mem.read(&*caller, ptr as u32 as usize, &mut buf).map_err(|_| wasmtime::format_err!("message out of bounds"))?;
    Ok(buf)
}

fn memory(caller: &mut Caller<'_, HostState>) -> HostResult<Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(m)) => Ok(m),
        _ => Err(wasmtime::format_err!("module does not export `memory`")),
    }
}

/// Reads an AssemblyScript string: UTF-16LE, byte length stored at `ptr - 4`.
fn read_as_string(mem: &[u8], ptr: i32) -> String {
    let ptr = ptr as u32 as usize;
    if ptr < 4 || ptr > mem.len() {
        return "<no message>".into();
    }
    let len = u32::from_le_bytes(mem[ptr - 4..ptr].try_into().unwrap()) as usize;
    let Some(bytes) = mem.get(ptr..ptr.saturating_add(len)) else {
        return "<bad string>".into();
    };
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}
