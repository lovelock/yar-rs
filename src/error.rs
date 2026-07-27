//! Error types for the Yar RPC framework

use thiserror::Error;

#[derive(Error, Debug)]
pub enum YarError {
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Pack error: {0}")]
    Pack(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Timeout")]
    Timeout,
    #[error("Invalid magic number")]
    InvalidMagic,
    #[error("Unsupported packager: {0}")]
    UnsupportedPackager(String),
    #[error("Method not found: {0}")]
    MethodNotFound(String),
}

pub type YarResult<T> = Result<T, YarError>;
