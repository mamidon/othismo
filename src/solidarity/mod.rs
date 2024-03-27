use std::result;
use wasmer::{CompileError, InstantiationError};

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
    Instantiation(InstantiationError)
}

pub type Result<T, E=Errors> = result::Result<T,E>;
#[derive(Debug)]
pub enum Errors {
    Solidarity(SolidarityError),
    Rusqlite(rusqlite::Error),
    Io(std::io::Error),
    Wasmer(WasmerError)
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