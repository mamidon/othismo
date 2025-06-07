use std::result;
use wasmbin::io::DecodeError;
use wasmer::{wasmparser::BinaryReaderError, CompileError, ExportError, InstantiationError, MemoryAccessError, RuntimeError};

pub mod image;
pub mod namespace;
pub mod execution;

#[derive(Debug)]
pub enum OthismoError {
    ImageAlreadyExists,
    ObjectAlreadyExists,
    ObjectDoesNotExist,
    ObjectNotFree,
    UnsupportedModuleDefinition(String)
}

#[derive(Debug)]
pub enum WasmerError {
    Compile(CompileError),
    Instantiation(InstantiationError),
    Export(ExportError),
    RuntimeError(RuntimeError),
    Memory(MemoryAccessError)
}

#[derive(Debug)]
pub enum WasmParserError {
    BinaryReaderError(BinaryReaderError)
}

#[derive(Debug)]
pub enum WasmBinError {
    DecodeError(DecodeError)
}

pub type Result<T, E=Errors> = result::Result<T,E>;
#[derive(Debug)]
pub enum Errors {
    Othismo(OthismoError),
    Rusqlite(rusqlite::Error),
    Io(std::io::Error),
    Wasmer(WasmerError),
    WasmParser(WasmParserError),
    WasmBin(WasmBinError),
    BsonSerialize(bson::ser::Error),
    BsonDeserialize(bson::de::Error)
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