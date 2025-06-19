use bson::{de, Document};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::result;
use std::sync::{Arc, MutexGuard};
use std::sync::{Mutex, TryLockError};
use std::task::Waker;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use wasmbin::io::DecodeError;
use wasmer::{
    wasmparser::BinaryReaderError, CompileError, ExportError, InstantiationError,
    MemoryAccessError, RuntimeError,
};

pub mod executors;
pub mod image;
pub mod namespace;

#[derive(Debug)]
pub enum OthismoError {
    ImageAlreadyExists,
    ObjectAlreadyExists,
    ObjectDoesNotExist,
    ObjectNotFree,
    UnsupportedModuleDefinition(String),
}

#[derive(Debug)]
pub enum WasmerError {
    Compile(CompileError),
    Instantiation(InstantiationError),
    Export(ExportError),
    RuntimeError(RuntimeError),
    Memory(MemoryAccessError),
}

#[derive(Debug)]
pub enum WasmParserError {
    BinaryReaderError(BinaryReaderError),
}

#[derive(Debug)]
pub enum WasmBinError {
    DecodeError(DecodeError),
}

pub type Result<T, E = Errors> = result::Result<T, E>;
#[derive(Debug)]
pub enum Errors {
    Othismo(OthismoError),
    Rusqlite(rusqlite::Error),
    Io(std::io::Error),
    Wasmer(WasmerError),
    WasmParser(WasmParserError),
    WasmBin(WasmBinError),
    BsonSerialize(bson::ser::Error),
    BsonDeserialize(bson::de::Error),
}

impl From<rusqlite::Error> for Errors {
    fn from(value: rusqlite::Error) -> Self {
        Errors::Rusqlite(value)
    }
}

impl From<std::io::Error> for Errors {
    fn from(value: std::io::Error) -> Self {
        Errors::Io(value)
    }
}

impl From<OthismoError> for Errors {
    fn from(value: OthismoError) -> Self {
        Errors::Othismo(value)
    }
}

impl From<CompileError> for Errors {
    fn from(value: CompileError) -> Self {
        Errors::Wasmer(WasmerError::Compile(value))
    }
}

impl From<InstantiationError> for Errors {
    fn from(value: InstantiationError) -> Self {
        Errors::Wasmer(WasmerError::Instantiation(value))
    }
}

impl From<ExportError> for Errors {
    fn from(value: ExportError) -> Self {
        Errors::Wasmer(WasmerError::Export(value))
    }
}

impl From<RuntimeError> for Errors {
    fn from(value: RuntimeError) -> Self {
        Errors::Wasmer(WasmerError::RuntimeError(value))
    }
}

impl From<MemoryAccessError> for Errors {
    fn from(value: MemoryAccessError) -> Self {
        Errors::Wasmer(WasmerError::Memory(value))
    }
}

impl From<BinaryReaderError> for Errors {
    fn from(value: BinaryReaderError) -> Self {
        Errors::WasmParser(WasmParserError::BinaryReaderError(value))
    }
}

impl From<DecodeError> for Errors {
    fn from(value: DecodeError) -> Self {
        Errors::WasmBin(WasmBinError::DecodeError(value))
    }
}

impl From<bson::ser::Error> for Errors {
    fn from(value: bson::ser::Error) -> Self {
        Errors::BsonSerialize(value)
    }
}

impl From<bson::de::Error> for Errors {
    fn from(value: bson::de::Error) -> Self {
        Errors::BsonDeserialize(value)
    }
}

pub struct Message {
    bytes: Vec<u8>,
}

impl Message {
    pub fn new(bytes: Vec<u8>) -> Self {
        Message { bytes }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn to_bson(&self) -> Document {
        bson::from_slice(&self.bytes).expect("Failed to convert message bytes to BSON")
    }
}

pub struct Channel<T> {
    pub tx: UnboundedSender<T>,
    pub rx: UnboundedReceiver<T>,
}

impl<T> Channel<T> {
    pub fn new() -> Channel<T> {
        let (tx, rx) = unbounded_channel();

        Channel { tx, rx }
    }

    pub fn split(self) -> (UnboundedSender<T>, UnboundedReceiver<T>) {
        (self.tx, self.rx)
    }
}

pub trait ProcessExecutor: Send + 'static {
    fn start(self, context: ProcessCtx) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

pub struct ProcessCtx {
    inbox: UnboundedReceiver<Message>,
    outbox: UnboundedSender<Message>,
    waker_slot: Arc<Mutex<Option<Waker>>>,
}

pub struct Process {
    inbox_tx: UnboundedSender<Message>,
    handle: JoinHandle<()>,
    waker: Option<Waker>,
    waker_slot: Arc<Mutex<Option<Waker>>>,
}

impl ProcessCtx {
    pub fn get_waker_slot(&self) -> Arc<Mutex<Option<Waker>>> {
        self.waker_slot.clone()
    }

    pub fn fill_waker_slot(&self, waker: Waker) -> () {
        self.waker_slot
            .lock()
            .map(|mut guard| guard.replace(waker))
            .unwrap();
    }
}
