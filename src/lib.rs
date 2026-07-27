//! Yar RPC framework - Rust implementation
//!
//! A concurrent RPC framework compatible with the original yar C implementation.

pub mod protocol;
pub mod pack;
pub mod request;
pub mod response;
pub mod logger;
pub mod client;
pub mod server;
pub mod error;

pub use protocol::{YarHeader, HEADER_SIZE, PROTOCOL_MAGIC, PROTOCOL_PERSISTENT, PROTOCOL_PING, PROTOCOL_LIST};
pub use pack::{Packager, Unpackager, YarData, YarDataType};
pub use request::YarRequest;
pub use response::YarResponse;
pub use client::YarClient;
pub use server::YarServer;
pub use error::{YarError, YarResult};

pub const VERSION: &str = "0.1.0";
pub const PACKAGER_NAME: &[u8; 8] = b"MSGPACK\0";
pub const PACKAGER_NAME_STR: &str = "MSGPACK";
