use std::result;
use wasmbin::io::DecodeError;
use wasmer::{wasmparser::BinaryReaderError, CompileError, ExportError, InstantiationError, RuntimeError};

pub mod image;

#[derive(Debug)]
pub enum SolidarityError {
    ImageAlreadyExists,
    ObjectAlreadyExists,
    ObjectDoesNotExist,
    ObjectNotFree
}

#[derive(Debug)]
pub enum WasmerError {
    Compile(CompileError),
    Instantiation(InstantiationError),
    Export(ExportError),
    RuntimeError(RuntimeError),
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
    Solidarity(SolidarityError),
    Rusqlite(rusqlite::Error),
    Io(std::io::Error),
    Wasmer(WasmerError),
    WasmParser(WasmParserError),
    WasmBin(WasmBinError),
    Bson(bson::ser::Error)
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

impl From<SolidarityError> for Errors {
    fn from(value: SolidarityError) -> Self {
        Errors::Solidarity(value)
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
        Errors::Bson(value)
    }
}