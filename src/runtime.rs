//! Instantiates a checked [`Program`] and runs its message loop.
//!
//! Delivery is synchronous and single-threaded: one FIFO queue of
//! `(target, sender, message)` entries, drained completely before the next
//! outside [`Event`] is taken. Objects send in two ways: `emit`, which the
//! image's routes direct, and `reply`, which goes back to whoever sent the
//! message being answered.
//!
//! Wasm objects get a [`Store`] each and an import set of exactly `env.abort`,
//! `real_talk.emit` and `real_talk.reply`; anything else a module asks for is
//! refused before it is instantiated.

use crate::check::{Kind, Program};
use crate::env::{self, EnvObject, Event, Out};
use crate::messages::MessageType;
use crate::wire::Message;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, Write};
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
    /// Check every message's fields against its declaration in transit.
    pub check_shapes: bool,
}

/// Every host function a module may import.
const ALLOWED_IMPORTS: &[(&str, &str)] = &[("env", "abort"), ("real_talk", "emit"), ("real_talk", "reply")];

/// Reply handle meaning "the image itself" (startup sends); replies to it are dropped.
const NOBODY: u32 = 0;

#[derive(Default)]
struct HostState {
    /// Messages the module sent during the current call, as BSON.
    outbox: Vec<Sent>,
    /// Reply handles: handle `h` (from 1) names instance `senders[h - 1]`.
    /// Only instances that have messaged this object ever get a handle, so a
    /// guessed handle can only reach something it could already reply to.
    senders: Vec<usize>,
    handles: HashMap<usize, u32>,
}

enum Sent {
    Emit(Vec<u8>),
    Reply(u32, Vec<u8>),
}

struct WasmObject {
    store: Store<HostState>,
    memory: Memory,
    inbox: TypedFunc<i32, i32>,
    deliver: TypedFunc<i32, ()>,
}

enum Object {
    Wasm(WasmObject),
    Env(EnvObject),
}

pub struct Runtime<'p> {
    program: &'p Program,
    objects: Vec<Object>,
    /// `(to, from, message)`; `from` is `None` for the image's startup sends.
    queue: VecDeque<(usize, Option<usize>, Message)>,
    input: Option<Box<dyn BufRead + Send>>,
    output: Box<dyn Write>,
    options: Options,
}

/// Runs `program` until no outside event can arrive any more (stdin is
/// exhausted and nothing is listening) and every queued message is delivered.
pub fn run(program: &Program, io: Io, options: Options) -> Result<()> {
    Runtime::new(program, io, options)?.run()
}

impl<'p> Runtime<'p> {
    /// Instantiates every object. Messages sent by module start functions are
    /// queued, not yet delivered.
    pub fn new(program: &'p Program, io: Io, options: Options) -> Result<Runtime<'p>> {
        let engine = Engine::default();
        let linker = host_linker(&engine)?;
        let mut rt = Runtime {
            program,
            objects: vec![],
            queue: VecDeque::new(),
            input: Some(io.input),
            output: io.output,
            options,
        };
        for (i, inst) in program.instances.iter().enumerate() {
            let obj = match &inst.kind {
                Kind::Wasm { protocol, module } => {
                    let bytes = std::fs::read(module).with_context(|| {
                        format!("instance `{}`: cannot read {} (did you compile {protocol}.ts?)", inst.name, module.display())
                    })?;
                    let module = Module::new(&engine, bytes)
                        .map_err(anyhow::Error::from)
                        .with_context(|| format!("instance `{}`: invalid module {}", inst.name, module.display()))?;
                    for imp in module.imports() {
                        if !ALLOWED_IMPORTS.contains(&(imp.module(), imp.name())) {
                            bail!(
                                "instance `{}`: module for {protocol} imports `{}.{}`, which the runtime does not provide",
                                inst.name,
                                imp.module(),
                                imp.name()
                            );
                        }
                    }
                    let mut store = Store::new(&engine, HostState::default());
                    let instance = linker
                        .instantiate(&mut store, &module)
                        .map_err(|e| anyhow!("instance `{}` failed to start: {e:#}", inst.name))?;
                    let memory = instance
                        .get_memory(&mut store, "memory")
                        .ok_or_else(|| anyhow!("instance `{}`: module does not export `memory`", inst.name))?;
                    let inbox = instance
                        .get_typed_func(&mut store, "rt_inbox")
                        .map_err(anyhow::Error::from)
                        .with_context(|| format!("instance `{}`: module must export rt_inbox(i32) -> i32", inst.name))?;
                    let deliver = instance
                        .get_typed_func(&mut store, "rt_deliver")
                        .map_err(anyhow::Error::from)
                        .with_context(|| format!("instance `{}`: module must export rt_deliver(i32)", inst.name))?;
                    Object::Wasm(WasmObject { store, memory, inbox, deliver })
                }
                kind => Object::Env(
                    EnvObject::new(kind)?
                        .ok_or_else(|| anyhow!("no environment object for {kind:?}"))
                        .with_context(|| format!("instance `{}`", inst.name))?,
                ),
            };
            rt.objects.push(obj);
            rt.flush_outbox(i)?;
        }
        Ok(rt)
    }

    pub fn run(mut self) -> Result<()> {
        // Startup sends go out once every object exists, before outside events.
        for (i, inst) in self.program.instances.iter().enumerate() {
            for msg in &inst.startup {
                self.queue.push_back((i, None, msg.clone()));
            }
        }
        self.drain()?;

        let (tx, rx) = mpsc::channel::<Event>();
        let stdins: Vec<usize> = (0..self.objects.len())
            .filter(|&i| matches!(self.objects[i], Object::Env(EnvObject::Stdin)))
            .collect();
        if !stdins.is_empty() {
            env::spawn_stdin(self.input.take().expect("input is taken once"), stdins, tx.clone());
        }
        for i in 0..self.objects.len() {
            if let Object::Env(e) = &mut self.objects[i] {
                let sent = e.start(i, &tx).with_context(|| format!("instance `{}`", self.program.instances[i].name))?;
                self.send_all(i, None, sent)?;
            }
        }
        // Once every source thread has finished, `rx` disconnects and we're done.
        drop(tx);
        self.drain()?;

        for ev in rx {
            let Object::Env(e) = &mut self.objects[ev.to] else { unreachable!("events only come from env objects") };
            let sent = e.on_event(ev.what);
            self.send_all(ev.to, None, sent)?;
            self.drain()?;
        }
        Ok(())
    }

    fn drain(&mut self) -> Result<()> {
        while let Some((to, from, msg)) = self.queue.pop_front() {
            self.deliver(to, from, msg)?;
        }
        Ok(())
    }

    fn deliver(&mut self, to: usize, from: Option<usize>, msg: Message) -> Result<()> {
        let name = &self.program.instances[to].name;
        match &mut self.objects[to] {
            Object::Env(e) => {
                let sent = e
                    .deliver(&msg, self.program, &mut self.output)
                    .with_context(|| format!("instance `{name}` failed handling `{}`", msg.type_name))?;
                self.send_all(to, from, sent)?;
            }
            Object::Wasm(obj) => {
                let bytes = msg.encode();
                let state = obj.store.data_mut();
                let handle = match from {
                    None => NOBODY,
                    Some(f) => *state.handles.entry(f).or_insert_with(|| {
                        state.senders.push(f);
                        state.senders.len() as u32
                    }),
                };
                let call = |obj: &mut WasmObject| -> Result<()> {
                    let ptr = obj.inbox.call(&mut obj.store, bytes.len() as i32)?;
                    obj.memory.write(&mut obj.store, ptr as u32 as usize, &bytes)?;
                    Ok(obj.deliver.call(&mut obj.store, handle as i32)?)
                };
                call(obj).map_err(|e| anyhow!("instance `{name}` failed handling `{}`: {e:#}", msg.type_name))?;
                self.flush_outbox(to)?;
            }
        }
        Ok(())
    }

    /// Sends what an environment object produced while handling a message from `from`.
    fn send_all(&mut self, me: usize, from: Option<usize>, sent: Vec<Out>) -> Result<()> {
        for out in sent {
            match out {
                Out::Emit(msg) => self.emit(me, msg)?,
                Out::Reply(msg) => self.reply(me, from, msg)?,
            }
        }
        Ok(())
    }

    /// Sends whatever Wasm instance `me` produced during its last call.
    fn flush_outbox(&mut self, me: usize) -> Result<()> {
        let Object::Wasm(obj) = &mut self.objects[me] else { return Ok(()) };
        let state = obj.store.data_mut();
        let sent = std::mem::take(&mut state.outbox);
        let senders = state.senders.clone();
        let name = &self.program.instances[me].name;
        for s in sent {
            let (to, bytes) = match s {
                Sent::Emit(bytes) => (None, bytes),
                Sent::Reply(NOBODY, bytes) => (Some(None), bytes),
                Sent::Reply(h, bytes) => match senders.get(h as usize - 1) {
                    Some(&i) => (Some(Some(i)), bytes),
                    None => bail!("instance `{name}` replied with unknown handle {h}"),
                },
            };
            let msg = Message::decode(&bytes).with_context(|| format!("instance `{name}` sent an undecodable message"))?;
            match to {
                None => self.emit(me, msg)?,
                Some(to) => self.reply(me, to, msg)?,
            }
        }
        Ok(())
    }

    /// Enqueues `msg` for every target the image routes `from`'s `msg.type_name` to.
    fn emit(&mut self, from: usize, msg: Message) -> Result<()> {
        let inst = &self.program.instances[from];
        if !inst.emits.contains(&msg.type_name) {
            bail!("instance `{}` emitted `{}`, but its protocol only emits {}", inst.name, msg.type_name, list(&inst.emits));
        }
        self.check_shape(from, &msg)?;
        let targets = self.program.routes.get(&(from, msg.type_name.clone())).cloned().unwrap_or_default();
        for t in targets {
            self.queue.push_back((t, Some(from), msg.clone()));
        }
        Ok(())
    }

    /// Enqueues `msg` back to `to`; a reply to the image (`None`) is dropped.
    fn reply(&mut self, from: usize, to: Option<usize>, msg: Message) -> Result<()> {
        let inst = &self.program.instances[from];
        if !inst.replies.contains(&msg.type_name) {
            bail!("instance `{}` replied `{}`, but its protocol only replies {}", inst.name, msg.type_name, list(&inst.replies));
        }
        self.check_shape(from, &msg)?;
        let Some(to) = to else { return Ok(()) };
        let target = &self.program.instances[to];
        if !target.accepts(&msg.type_name) {
            bail!("instance `{}` replied `{}` to `{}`, which doesn't accept it", inst.name, msg.type_name, target.name);
        }
        self.queue.push_back((to, Some(from), msg));
        Ok(())
    }

    fn check_shape(&self, from: usize, msg: &Message) -> Result<()> {
        if self.options.check_shapes {
            check_shape(&self.program.messages, msg)
                .with_context(|| format!("instance `{}` sent a malformed `{}`", self.program.instances[from].name, msg.type_name))?;
        }
        Ok(())
    }
}

fn list(types: &[String]) -> String {
    if types.is_empty() { "nothing".into() } else { types.join(", ") }
}

fn check_shape(messages: &BTreeMap<String, MessageType>, msg: &Message) -> Result<()> {
    msg.check_shape(&messages[&msg.type_name])
}

fn host_linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    linker.func_wrap(
        "real_talk",
        "emit",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> wasmtime::Result<()> {
            let buf = read_bytes(&mut caller, ptr, len)?;
            caller.data_mut().outbox.push(Sent::Emit(buf));
            Ok(())
        },
    )?;
    linker.func_wrap(
        "real_talk",
        "reply",
        |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32, len: i32| -> wasmtime::Result<()> {
            let buf = read_bytes(&mut caller, ptr, len)?;
            caller.data_mut().outbox.push(Sent::Reply(handle as u32, buf));
            Ok(())
        },
    )?;
    linker.func_wrap(
        "env",
        "abort",
        |mut caller: Caller<'_, HostState>, msg: i32, file: i32, line: i32, col: i32| -> wasmtime::Result<()> {
            let mem = memory(&mut caller)?;
            let msg = read_as_string(mem.data(&caller), msg);
            let file = read_as_string(mem.data(&caller), file);
            Err(wasmtime::format_err!("abort: {msg} ({file}:{line}:{col})"))
        },
    )?;
    Ok(linker)
}

fn read_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> wasmtime::Result<Vec<u8>> {
    let mem = memory(caller)?;
    let mut buf = vec![0; len as u32 as usize];
    mem.read(&*caller, ptr as u32 as usize, &mut buf).map_err(|_| wasmtime::format_err!("message out of bounds"))?;
    Ok(buf)
}

fn memory(caller: &mut Caller<'_, HostState>) -> wasmtime::Result<Memory> {
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
