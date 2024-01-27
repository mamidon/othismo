use std::result;
use wasmer::CompileError;
use crate::solidarity;

pub mod image;

#[derive(Debug)]
pub enum Error {
    ImageAlreadyExists
}

pub type Result<T, E=Errors> = result::Result<T,E>;
#[derive(Debug)]
pub enum Errors {
    Solidarity(Error),
    Rusqlite(rusqlite::Error),
    Io(std::io::Error),
    Wasmer(wasmer::CompileError)
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

impl From<Error> for Errors {
    fn from(value: solidarity::Error) -> Self {
        Errors::Solidarity(value)
    }
}

impl From<CompileError> for Errors {
    fn from(value: CompileError) -> Self {
        Errors::Wasmer(value)
    }
}
