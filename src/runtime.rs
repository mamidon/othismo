//! Instantiates a checked [`Program`] and runs its message loop.
//!
//! Delivery is synchronous and single-threaded: one FIFO queue of
//! `(target instance, message)` pairs. Wasm objects get a [`Store`] each and
//! an import set of exactly `env.abort` and `real_talk.send`; anything else a
//! module asks for is refused before it is instantiated.

use crate::check::{Kind, Program};
use crate::messages::MessageType;
use crate::render;
use crate::wire::Message;
use anyhow::{Context, Result, anyhow, bail};
use bson::doc;
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, Write};
use wasmtime::{Caller, Engine, Extern, Linker, Memory, Module, Store, TypedFunc};

/// The outside world, as seen by the environmental objects. Tests swap in
/// in-memory buffers; the image doesn't change.
pub struct Io {
    pub input: Box<dyn BufRead>,
    pub output: Box<dyn Write>,
}

impl Io {
    pub fn std() -> Io {
        Io { input: Box::new(std::io::stdin().lock()), output: Box::new(std::io::stdout()) }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Options {
    /// Check every message's fields against its declaration in transit.
    pub check_shapes: bool,
}

/// Every host function a module may import.
const ALLOWED_IMPORTS: &[(&str, &str)] = &[("env", "abort"), ("real_talk", "send")];

#[derive(Default)]
struct HostState {
    /// Messages the module sent during the current call: (port, BSON).
    outbox: Vec<(u32, Vec<u8>)>,
}

struct WasmObject {
    store: Store<HostState>,
    memory: Memory,
    inbox: TypedFunc<i32, i32>,
    deliver: TypedFunc<(), ()>,
}

enum Object {
    Wasm(WasmObject),
    Stdin,
    Stdout,
}

pub struct Runtime<'p> {
    program: &'p Program,
    objects: Vec<Object>,
    queue: VecDeque<(usize, Message)>,
    io: Io,
    options: Options,
}

/// Runs `program` until stdin is exhausted and every queued message is delivered.
pub fn run(program: &Program, io: Io, options: Options) -> Result<()> {
    Runtime::new(program, io, options)?.run()
}

impl<'p> Runtime<'p> {
    /// Instantiates every object. Messages sent by module start functions are
    /// queued, not yet delivered.
    pub fn new(program: &'p Program, io: Io, options: Options) -> Result<Runtime<'p>> {
        let engine = Engine::default();
        let linker = host_linker(&engine)?;
        let mut rt = Runtime { program, objects: vec![], queue: VecDeque::new(), io, options };
        for (i, inst) in program.instances.iter().enumerate() {
            let obj = match &inst.kind {
                Kind::Stdin => Object::Stdin,
                Kind::Stdout => Object::Stdout,
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
                        .with_context(|| format!("instance `{}`: module must export rt_deliver()", inst.name))?;
                    Object::Wasm(WasmObject { store, memory, inbox, deliver })
                }
            };
            rt.objects.push(obj);
            rt.flush_outbox(i)?;
        }
        Ok(rt)
    }

    pub fn run(mut self) -> Result<()> {
        // Startup sends go out after everything is wired, before external input.
        for (i, inst) in self.program.instances.iter().enumerate() {
            for msg in &inst.startup {
                self.queue.push_back((i, msg.clone()));
            }
        }
        self.drain()?;

        let stdins: Vec<usize> =
            (0..self.objects.len()).filter(|&i| matches!(self.objects[i], Object::Stdin)).collect();
        if stdins.is_empty() {
            return Ok(());
        }
        // With several stdin instances, lines are handed out round-robin.
        for &from in stdins.iter().cycle() {
            let mut line = String::new();
            if self.io.input.read_line(&mut line).context("reading stdin")? == 0 {
                break;
            }
            let text = line.strip_suffix('\n').unwrap_or(&line);
            let text = text.strip_suffix('\r').unwrap_or(text);
            let msg = Message { type_name: crate::check::LINE.into(), fields: doc! { "text": text } };
            self.route(from, 0, msg)?;
            self.drain()?;
        }
        Ok(())
    }

    fn drain(&mut self) -> Result<()> {
        while let Some((to, msg)) = self.queue.pop_front() {
            self.deliver(to, msg)?;
        }
        Ok(())
    }

    fn deliver(&mut self, to: usize, msg: Message) -> Result<()> {
        let name = &self.program.instances[to].name;
        match &mut self.objects[to] {
            Object::Stdin => unreachable!("the checker rejects wiring into {}", crate::check::STDIN),
            Object::Stdout => {
                let line = render::render(&msg, &self.program.messages);
                writeln!(self.io.output, "{line}").and_then(|_| self.io.output.flush()).context("writing stdout")?;
            }
            Object::Wasm(obj) => {
                let bytes = msg.encode();
                let call = |obj: &mut WasmObject| -> Result<()> {
                    let ptr = obj.inbox.call(&mut obj.store, bytes.len() as i32)?;
                    obj.memory.write(&mut obj.store, ptr as u32 as usize, &bytes)?;
                    Ok(obj.deliver.call(&mut obj.store, ())?)
                };
                call(obj).map_err(|e| anyhow!("instance `{name}` failed handling `{}`: {e:#}", msg.type_name))?;
                self.flush_outbox(to)?;
            }
        }
        Ok(())
    }

    /// Routes whatever instance `from` sent during its last call.
    fn flush_outbox(&mut self, from: usize) -> Result<()> {
        let Object::Wasm(obj) = &mut self.objects[from] else { return Ok(()) };
        let sent = std::mem::take(&mut obj.store.data_mut().outbox);
        for (port, bytes) in sent {
            let msg = Message::decode(&bytes)
                .with_context(|| format!("instance `{}` sent an undecodable message", self.program.instances[from].name))?;
            self.route(from, port, msg)?;
        }
        Ok(())
    }

    /// Enqueues `msg` for every target of `from`'s out port `port`, after
    /// checking the port exists and is declared to carry this type.
    fn route(&mut self, from: usize, port: u32, msg: Message) -> Result<()> {
        let inst = &self.program.instances[from];
        let Some(p) = inst.ports.get(port as usize) else {
            bail!("instance `{}` sent on port #{port}, but its protocol declares {} out port(s)", inst.name, inst.ports.len());
        };
        if !p.types.contains(&msg.type_name) {
            bail!(
                "instance `{}` sent `{}` on port `{}`, which only carries {}",
                inst.name,
                msg.type_name,
                p.name,
                p.types.join(", ")
            );
        }
        if self.options.check_shapes {
            check_shape(&self.program.messages, &msg)
                .with_context(|| format!("instance `{}` sent a malformed message on `{}`", inst.name, p.name))?;
        }
        for &t in &p.targets {
            self.queue.push_back((t, msg.clone()));
        }
        Ok(())
    }
}

fn check_shape(messages: &BTreeMap<String, MessageType>, msg: &Message) -> Result<()> {
    msg.check_shape(&messages[&msg.type_name])
}

fn host_linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    linker.func_wrap(
        "real_talk",
        "send",
        |mut caller: Caller<'_, HostState>, port: i32, ptr: i32, len: i32| -> wasmtime::Result<()> {
            let mem = memory(&mut caller)?;
            let mut buf = vec![0; len as u32 as usize];
            mem.read(&caller, ptr as u32 as usize, &mut buf).map_err(|_| wasmtime::format_err!("send: message out of bounds"))?;
            caller.data_mut().outbox.push((port as u32, buf));
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
