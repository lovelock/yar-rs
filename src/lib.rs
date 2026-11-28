#![doc = include_str!("../README.md")]

//! Yar RPC framework - Rust implementation
//!
//! 与原 Yar C 实现线上协议完全兼容的 RPC 框架。
//! 完整的中文用户手册见仓库 `docs/` 目录（根 README 有索引）。
//!
//! # 三个典型入口
//!
//! - 服务端：[`YarServer::new`] + [`YarServer::register_handler`] + [`YarServer::run`]（Tokio 异步）
//! - 客户端：[`YarClient::connect`] + [`YarClient::call`]（同步阻塞，可选持久连接）
//! - 打包：[`Packager`]（增量打包） / [`Unpackager`]（解析为 [`YarData`] 树）
//!
//! # 打包/解析往返示例
//!
//! ```
//! use yar_rs::{Packager, Unpackager, YarData};
//!
//! // 构造 {"name" => "yar", "nums" => [1, 2, 3]}
//! let mut pk = Packager::start_map(2).unwrap();
//! pk.push_string(b"name").unwrap();
//! pk.push_string(b"yar").unwrap();
//! pk.push_string(b"nums").unwrap();
//! pk.push_array(3).unwrap();
//! pk.push_long(1).unwrap();
//! pk.push_long(2).unwrap();
//! pk.push_long(3).unwrap();
//!
//! let data: YarData = Unpackager::new(pk.as_bytes()).unwrap().into_data();
//! let map = data.as_map().unwrap();
//! assert_eq!(map[0].1.as_str(), Some("yar"));
//! assert_eq!(map[1].1.as_array().unwrap().len(), 3);
//! // 注意：小正数解包为 Ulong，读取整数用 as_integer()
//! assert_eq!(map[1].1.as_array().unwrap()[0].as_integer(), Some(1));
//! ```
//!
//! # 最小 RPC 调用示例
//!
//! 不需要完整网络即可演示核心数据流（服务端 handler 收到请求 → 构造响应）：
//!
//! ```
//! use yar_rs::{Packager, YarRequest, YarResponse};
//!
//! // ① 客户端侧构造请求：add(10, 20)
//! let mut a = Packager::new(); a.push_long(10).unwrap();
//! let mut b = Packager::new(); b.push_long(20).unwrap();
//! let mut nums = Packager::start_array(2).unwrap();
//! nums.push_packager(&a).unwrap();
//! nums.push_packager(&b).unwrap();
//!
//! let mut req = YarRequest::new(1000, "add");
//! req.set_parameters(nums);
//!
//! // ② 服务端 handler：读参数数组，返回和
//! let resp = {
//!     let arr = req.get_parameters().and_then(|p| p.as_array()).unwrap_or(&[]);
//!     let sum: i64 = arr.iter().filter_map(|v| v.as_integer()).sum();
//!     let mut pk = Packager::new();
//!     pk.push_long(sum).unwrap();
//!     YarResponse::success(req.id(), pk)
//! };
//!
//! // ③ 真正的网络传输由 YarClient/YarServer 完成（wire 格式见 protocol 模块）
//! assert_eq!(resp.get_status(), 0);
//! ```

#[doc(hidden)]
pub mod cli;
pub mod client;
pub mod error;
pub mod logger;
pub mod pack;
#[cfg(feature = "profiler")]
pub mod profiler;
pub mod protocol;
pub mod request;
pub mod response;
pub mod server;

pub use client::{YarClient, DEFAULT_MAX_RESPONSE_SIZE};
pub use error::{YarError, YarResult};
pub use pack::{Packager, Unpackager, YarData, YarDataType};
pub use protocol::{
    YarHeader, HEADER_SIZE, PROTOCOL_LIST, PROTOCOL_MAGIC, PROTOCOL_PERSISTENT, PROTOCOL_PING,
};
pub use request::YarRequest;
pub use response::YarResponse;
pub use server::{
    YarServer, DEFAULT_MAX_INFLIGHT, DEFAULT_MAX_REQUEST_SIZE, DEFAULT_REQUEST_TIMEOUT,
    DEFAULT_WRITE_TIMEOUT,
};

/// Crate version, taken from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Packager name embedded in every Yar packet.
pub const PACKAGER_NAME: &[u8; 8] = b"MSGPACK\0";
/// Packager name without the trailing NUL.
pub const PACKAGER_NAME_STR: &str = "MSGPACK";
