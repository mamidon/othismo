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
//! `serves`, its answers must be of the type its entry promises, and every
//! body must be a declared type with exactly its fields. A request that
//! doesn't fit is answered with a status and never arrives; an answer that
//! doesn't fit, or a malformed body, is its sender's fault and stops the run.
//!
//! Wasm objects get a [`Store`] each and an import set of exactly `env.abort`
//! and the `othismo` functions in [`ALLOWED_IMPORTS`]; anything else a
//! module asks for is refused before it is instantiated.

use crate::check::{Kind, Program};
use crate::pack::Pack;
use crate::env::{self, EnvObject, Event, Reply};
use crate::iface::{self, Answer};
use crate::messages::MessageType;
use crate::ns::{self, Namespace, Resolved, Status, Verb};
use crate::wire::Message;
use anyhow::{Context, Result, anyhow, bail};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
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

#[derive(Debug, Default, Clone)]
pub struct Options {
    /// Don't hold objects to their interfaces, or bodies to their types.
    pub no_checks: bool,
}

/// Every host function a module may import.
pub const ALLOWED_IMPORTS: &[(&str, &str)] = &[
    ("env", "abort"),
    ("othismo", "call"),
    ("othismo", "reply"),
    ("othismo", "forward"),
    ("othismo", "cancel"),
    ("othismo", "bind"),
    ("othismo", "unmount"),
    ("othismo", "expose"),
];

/// Who made a request, and so where its replies go.
#[derive(Debug, Clone)]
enum Requester {
    /// A startup request, described for warnings. Replies are dropped.
    Image(String),
    /// Ticket 0 means no answer is wanted.
    Wasm { inst: usize, ticket: u32 },
}

/// A request being served.
struct Pending {
    caller: Requester,
    verb: Verb,
    /// Kept so the request can be forwarded.
    body: Option<Message>,
    /// The caller gave up; replies are dropped.
    cancelled: bool,
    /// What the server's interface promises to answer, unless checks are off.
    promise: Option<Promise>,
}

struct Promise {
    /// The request, for errors: `read /hits`.
    what: String,
    answer: Option<Answer>,
}

enum Delivery {
    Serve { to: usize, req: u32, verb: Verb, path: String, body: Option<Message> },
    Answer { to: usize, ticket: u32, status: Status, body: Option<Message>, done: bool },
    Cancel { to: usize, req: u32 },
}

/// The state host functions work on. Shared between the runtime and every
/// Wasm store, and only ever borrowed briefly, never across a call into Wasm.
struct Shared {
    names: Vec<String>,
    ns: Namespace,
    messages: BTreeMap<String, MessageType>,
    /// Each instance's interface.
    interfaces: Vec<iface::Object>,
    checks: bool,
    queue: VecDeque<Delivery>,
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
    fn resolve(&self, inst: usize, path: &str, verb: Verb, body: Option<&Message>) -> Result<Resolved, Status> {
        let r = self.ns.resolve(inst, path)?;
        if r.read_only && !verb.is_safe() {
            return Err(Status::ReadOnly);
        }
        if self.checks {
            let (at, rest) = self.ns.mount_of(inst, path).ok_or(Status::NotFound)?;
            let need = self.interfaces[inst].need(&at).ok_or(Status::NotFound)?;
            let entry = need.entry(&rest).ok_or(Status::NotFound)?;
            entry.takes(verb, body.map(|b| b.type_name.as_str()))?;
        }
        Ok(r)
    }

    /// Makes a request of wherever `to` resolved; failures to resolve, and
    /// requests the server's interface doesn't take, are answered straight away.
    fn request(&mut self, caller: Requester, to: Result<Resolved, Status>, verb: Verb, body: Option<Message>) {
        let r = match to {
            Ok(r) => r,
            Err(status) => return self.answer(&caller, status, None, true),
        };
        let promise = if self.checks {
            let entry = self.interfaces[r.inst].serving(&r.path).ok_or(Status::NotFound);
            match entry.and_then(|e| e.takes(verb, body.as_ref().map(|b| b.type_name.as_str()))) {
                Ok(served) => Some(Promise { what: format!("{verb} {}", r.path), answer: served.answer.clone() }),
                Err(status) => return self.answer(&caller, status, None, true),
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
        self.queue.push_back(Delivery::Serve { to: r.inst, req, verb, path: r.path, body });
    }

    fn answer(&mut self, caller: &Requester, status: Status, body: Option<Message>, done: bool) {
        match *caller {
            Requester::Image(ref what) => {
                if status != Status::Ok {
                    eprintln!("warning: startup request `{what}` failed: {status}");
                }
            }
            Requester::Wasm { ticket: 0, .. } => {}
            Requester::Wasm { inst, ticket } => {
                if done {
                    self.calls.remove(&(inst, ticket));
                }
                self.queue.push_back(Delivery::Answer { to: inst, ticket, status, body, done });
            }
        }
    }

    /// `server` replies to its request `req`.
    fn reply(&mut self, server: usize, req: u32, status: Status, body: Option<Message>, done: bool) -> Result<()> {
        if let Some(b) = &body {
            self.check_shape(server, b)?;
        }
        let Some(p) = self.requests.get(&(server, req)) else {
            bail!("instance `{}` replied to unknown request {req}", self.names[server]);
        };
        if let Some(promise) = &p.promise {
            keeps(&self.names[server], promise, body.as_ref(), done)?;
        }
        let (caller, cancelled) = (p.caller.clone(), p.cancelled);
        if done {
            self.requests.remove(&(server, req));
        }
        if !cancelled {
            self.answer(&caller, status, body, done);
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
        let to = self.resolve(server, path, p.verb, p.body.as_ref());
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

    fn check_shape(&self, from: usize, msg: &Message) -> Result<()> {
        if !self.checks {
            return Ok(());
        }
        let name = &self.names[from];
        let ty = self.messages.get(&msg.type_name).ok_or_else(|| {
            anyhow!("instance `{name}` sent `{}`, which no header in the image declares", msg.type_name)
        })?;
        msg.check_shape(ty).with_context(|| format!("instance `{name}` sent a malformed `{}`", msg.type_name))
    }
}

/// Whether an answer from `server` keeps what its interface promised.
fn keeps(server: &str, promise: &Promise, body: Option<&Message>, done: bool) -> Result<()> {
    let what = &promise.what;
    match (&promise.answer, body) {
        (None, Some(b)) => {
            bail!("instance `{server}` answered `{what}` with a `{}`, but its interface promises a status alone", b.type_name)
        }
        (Some(a), Some(b)) if a.ty != iface::OPAQUE && a.ty != b.type_name => {
            bail!("instance `{server}` answered `{what}` with a `{}`, but its interface promises `{}`", b.type_name, a.ty)
        }
        _ => {}
    }
    if !done && !promise.answer.as_ref().is_some_and(|a| a.stream) {
        bail!("instance `{server}` answered `{what}` more than once, but its interface promises one answer");
    }
    Ok(())
}

struct HostState {
    me: usize,
    shared: Rc<RefCell<Shared>>,
}

struct WasmObject {
    store: Store<HostState>,
    memory: Memory,
    inbox: TypedFunc<i32, i32>,
    serve: TypedFunc<(i32, i32, i32, i32), ()>,
    answer: TypedFunc<(i32, i32, i32, i32), ()>,
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
        let mut ns = Namespace::new(n);
        for (i, inst) in program.instances.iter().enumerate() {
            for m in &inst.mounts {
                ns.mount(i, &m.at, m.target, m.read_only);
            }
        }
        let interfaces = program
            .instances
            .iter()
            .map(|i| program.interfaces.get(&i.type_name).cloned().ok_or_else(|| anyhow!("instance `{}` has no interface", i.name)))
            .collect::<Result<_>>()?;
        let shared = Rc::new(RefCell::new(Shared {
            names: program.instances.iter().map(|i| i.name.clone()).collect(),
            ns,
            messages: program.messages.clone(),
            interfaces,
            checks: !options.no_checks,
            queue: VecDeque::new(),
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
                Kind::Wasm { type_name } => {
                    let state = HostState { me: i, shared: shared.clone() };
                    let module = pack.blob(&pack.modules[type_name]);
                    Object::Wasm(Box::new(instantiate(&engine, &linker, &inst.name, type_name, module, state)?))
                }
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

    pub fn run(mut self) -> Result<()> {
        // The image's startup requests, then every module's start().
        for (i, inst) in self.program.instances.iter().enumerate() {
            for s in &inst.startup {
                let what = format!("{} {} {}", s.verb.keyword(), inst.name, s.path);
                let to = Ok(Resolved { inst: i, path: s.path.clone(), read_only: false });
                let body = s.body.as_ref().map(|b| b.message().cloned()).transpose()?;
                self.shared.borrow_mut().request(Requester::Image(what), to, s.verb, body);
            }
        }
        self.drain()?;
        for i in 0..self.objects.len() {
            if let Object::Wasm(obj) = &mut self.objects[i] {
                obj.start.call(&mut obj.store, ()).map_err(|e| self.failed(i, "starting", e))?;
            }
        }
        self.drain()?;

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
            Delivery::Serve { to, req, verb, path, body } => {
                let doing = format!("serving `{} {path}`", verb.keyword());
                match &mut self.objects[to] {
                    Object::Env(e) => {
                        let messages = &self.program.messages;
                        let replies = e
                            .serve(req, verb, &path, body.as_ref(), &mut self.output, messages)
                            .map_err(|e| self.failed(to, &doing, e))?;
                        self.send_replies(to, replies)?;
                    }
                    Object::Wasm(obj) => {
                        let mut bytes = path.as_bytes().to_vec();
                        let body = body.map(|b| b.encode()).unwrap_or_default();
                        bytes.extend_from_slice(&body);
                        let args = (req as i32, verb as i32, path.len() as i32, body.len() as i32);
                        let res = obj.fill_inbox(&bytes).and_then(|_| Ok(obj.serve.call(&mut obj.store, args)?));
                        res.map_err(|e| self.failed(to, &doing, e))?;
                    }
                }
            }
            Delivery::Answer { to, ticket, status, body, done } => {
                let Object::Wasm(obj) = &mut self.objects[to] else { unreachable!("only Wasm objects make calls") };
                let body = body.map(|b| b.encode()).unwrap_or_default();
                let args = (ticket as i32, status as i32, body.len() as i32, done as i32);
                let res = obj.fill_inbox(&body).and_then(|_| Ok(obj.answer.call(&mut obj.store, args)?));
                res.map_err(|e| self.failed(to, "handling an answer", e))?;
            }
            Delivery::Cancel { to, req } => match &mut self.objects[to] {
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
            shared.reply(from, r.req, r.status, r.body, r.done)?;
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
    let serve = export("rt_serve", "(i32, i32, i32, i32)")?;
    let answer = export("rt_answer", "(i32, i32, i32, i32)")?;
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

/// An optional body from module memory: length 0 means none.
fn read_body(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> HostResult<Option<Message>> {
    if len == 0 {
        return Ok(None);
    }
    let me = caller.data().me;
    let bytes = read_bytes(caller, ptr, len)?;
    let shared = caller.data().shared.clone();
    let shared = shared.borrow();
    let msg = Message::decode(&bytes)
        .with_context(|| format!("instance `{}` sent an undecodable message", shared.names[me]))
        .map_err(host_err)?;
    shared.check_shape(me, &msg).map_err(host_err)?;
    Ok(Some(msg))
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
        |mut caller: Caller<'_, HostState>, verb: i32, path: i32, path_len: i32, body: i32, body_len: i32, expect: i32| {
            let me = caller.data().me;
            let path = read_path(&mut caller, path, path_len)?;
            let body = read_body(&mut caller, body, body_len)?;
            let mut shared = caller.data().shared.borrow_mut();
            let verb = Verb::from_i32(verb).ok_or_else(|| wasmtime::format_err!("call with unknown verb {verb}"))?;
            let ticket = if expect != 0 {
                shared.next_ticket[me] += 1;
                shared.next_ticket[me]
            } else {
                0
            };
            // Only the operator controls; an object's control never leaves it.
            let to = match path {
                _ if verb == Verb::Control => Err(Status::Forbidden),
                None => Err(Status::Invalid),
                Some(p) => shared.resolve(me, &p, verb, body.as_ref()),
            };
            shared.request(Requester::Wasm { inst: me, ticket }, to, verb, body);
            HostResult::Ok(ticket as i32)
        },
    )?;
    linker.func_wrap(
        "othismo",
        "reply",
        |mut caller: Caller<'_, HostState>, req: i32, status: i32, body: i32, body_len: i32, done: i32| -> HostResult<()> {
            let me = caller.data().me;
            let body = read_body(&mut caller, body, body_len)?;
            let status = Status::from_i32(status).ok_or_else(|| wasmtime::format_err!("reply with unknown status {status}"))?;
            let mut shared = caller.data().shared.borrow_mut();
            shared.reply(me, req as u32, status, body, done != 0).map_err(host_err)
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
