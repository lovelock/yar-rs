# yar-rs

[![crates.io](https://img.shields.io/crates/v/yar-rs.svg)](https://crates.io/crates/yar-rs)
[![docs.rs](https://docs.rs/yar-rs/badge.svg)](https://docs.rs/yar-rs)
[![CI](https://github.com/lovelock/yar-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/lovelock/yar-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/lovelock/yar-rs/blob/master/LICENSE)

Rust implementation of the [Yar](https://github.com/laruence/yar) RPC framework.

(see also: [Yar PHP framework](https://github.com/laruence/yar), [Yar Java framework](https://github.com/weibocom/yar-java))

`yar-rs` is an idiomatic **async Rust** rewrite of the original Yar C
framework: it speaks the same wire protocol, so a `yar-rs` server can serve
requests from Yar clients written in PHP, Java, C, or Rust — and vice versa.

## Documentation (中文用户指南)

完整的中文用户手册位于 [docs/](https://github.com/lovelock/yar-rs/blob/master/docs/README.md)：

- [快速上手](https://github.com/lovelock/yar-rs/blob/master/docs/getting-started.md)
- [服务端指南](https://github.com/lovelock/yar-rs/blob/master/docs/server.md)
- [客户端指南](https://github.com/lovelock/yar-rs/blob/master/docs/client.md)
- [数据模型](https://github.com/lovelock/yar-rs/blob/master/docs/data.md)
- [协议与跨语言互通](https://github.com/lovelock/yar-rs/blob/master/docs/protocol.md)
- [错误与日志](https://github.com/lovelock/yar-rs/blob/master/docs/errors-logging.md)
- [FAQ 与 C→Rust 迁移对照](https://github.com/lovelock/yar-rs/blob/master/docs/faq.md)

指南中的示例由 `tests/doc_examples.rs` 和 doctest 覆盖；`cargo test` 会校验
可编译的片段。

## Features

- **Wire-compatible** with the classic Yar protocol: 82-byte big-endian
  header (version 0, magic `0x80DFEC60`), followed by a packager name and a
  msgpack-encoded body.
- **Client** (`YarClient`): blocking API with `connect()` / `call()` /
  `close()`, persistent connections, connect/read/write timeouts, a 1-second
  default (as C), bounded response sizes, `tcp://` or `host:port`, and
  `call_raw()` / `build_request_packet()` for pre-packed hot paths.
- **Server** (`YarServer`): Tokio-based async server, register handlers by
  method name, bounded in-flight connections, idle and total request
  deadlines, TCP or Unix-domain sockets (Unix only), `tcp://` hostnames, a
  10 MiB request cap, and a per-request OKEY/ERROR access log (default on,
  as C).
- **Pack / Unpack** (`Packager`, `YarData`): msgpack based pack/unpack with
  the same data model as the C version (null, bool, long, ulong, double,
  string, array, map), with limits against malicious nesting and lengths.
- **Request / Response**: exact binary format of `yar_request` /
  `yar_response` (id, method, parameters / status, error, retval);
  server-generated errors use status 8 like C.
- **Logger** (`YarLogger`): file/stderr/pipe logger with the same levels and
  the `OKEY` access-log level as the C version.

## Install

```toml
[dependencies]
yar-rs = "0.1"
```

The server uses Tokio; add the runtime features you need:

```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Build from source:

```sh
cargo build --release
```

## Example

Complete runnable examples live in the
[`examples/`](https://github.com/lovelock/yar-rs/tree/master/examples) directory:

```sh
cargo run --example cross_server   # in one terminal
cargo run --example cross_client   # in another terminal
```

### Server

```rust,no_run
use yar_rs::{Packager, YarResponse, YarServer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = YarServer::new("127.0.0.1:9501");

    // Handlers are synchronous closures returning a YarResponse.
    server
        .register_handler("echo", |req| {
            let mut pk = Packager::new();
            pk.push_string(b"echo:ok").unwrap();
            YarResponse::success(req.id(), pk)
        })
        .await;

    server.run().await?;
    Ok(())
}
```

### Client

```rust,no_run
use yar_rs::{Packager, YarClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = YarClient::connect("127.0.0.1:9501")?;

    let mut a = Packager::new();
    a.push_long(10)?;
    let mut b = Packager::new();
    b.push_long(20)?;

    let response = client.call("add", vec![a, b])?;
    if response.get_status() == 0 {
        println!("result = {:?}", response.get_response());
    }
    client.close();
    Ok(())
}
```

## Tests

```sh
cargo test
cargo test --all-features   # includes the profiler tooling
```

The test suite includes unit tests for the protocol header, request/response
packing, and the packager; wire-format compatibility vectors from the C
implementation; client↔server integration tests over real TCP and Unix
sockets; and security regressions for malformed msgpack and oversized
responses.

## Profiler (C vs Rust benchmark)

The benchmark that compares this implementation with the original C one lives
in [`profile/`](https://github.com/lovelock/yar-rs/tree/master/profile) (not part
of the published crate). It runs a k6-style matrix with one persistent
connection per virtual user and reports attempt QPS, **successful QPS
(`ok_qps`)**, ok/fail, p50/p95/p99/p99.9, server CPU share, memory peak and
CFS throttling, plus the load generator's CPU share.

Both servers run the same 3-second idle timeout and with per-request access
logging off by default (the C benchmark build gates its OKEY log behind an
environment variable), so neither side pays log I/O the other avoids. Every
stage connects all VUs and warms up before measuring, and the profiler builds
the request packet once per stage (`call_raw`) instead of repacking it per
call. See
[`profile/README.md`](https://github.com/lovelock/yar-rs/blob/master/profile/README.md)
for the parity rules, report columns and the measured matrix.

## License

Apache-2.0 (see [LICENSE](https://github.com/lovelock/yar-rs/blob/master/LICENSE)).
Yar C framework attribution is recorded in
[NOTICE](https://github.com/lovelock/yar-rs/blob/master/NOTICE).
