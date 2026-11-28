# yar-rs

[![crates.io](https://img.shields.io/crates/v/yar-rs.svg)](https://crates.io/crates/yar-rs)
[![docs.rs](https://docs.rs/yar-rs/badge.svg)](https://docs.rs/yar-rs)
[![CI](https://github.com/lovelock/yar-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/lovelock/yar-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/lovelock/yar-rs/blob/master/LICENSE)

`yar-rs` is a Rust implementation of the Yar RPC protocol. It is an
independent project: the async server, blocking client, data model and
documentation are designed for Rust, while the wire format stays compatible
with the original C framework so it can interoperate with Yar clients and
servers written in C, PHP, Java or Rust.

The C implementation remains the reference for the wire protocol:
[laruence/yar-c](https://github.com/laruence/yar-c) and the PHP extension
[laruence/yar](https://github.com/laruence/yar). This project is not a fork of
the C code or of its documentation.

## Documentation

- **English guide** — [docs/README.md](https://github.com/lovelock/yar-rs/blob/master/docs/README.md)
- **中文文档** — [docs/zh-CN/README.md](https://github.com/lovelock/yar-rs/blob/master/docs/zh-CN/README.md)
- **API reference** — <https://docs.rs/yar-rs>

The guides cover the design, server, client, data model, wire protocol,
errors/logging and FAQ. Runnable examples are exercised by
`tests/doc_examples.rs`.

## Features

- **Wire-compatible** with the classic Yar protocol: 82-byte big-endian
  header (version 0, magic `0x80DFEC60`), followed by an 8-byte packager name
  and a msgpack-encoded body.
- **Server** (`YarServer`): Tokio-based async server, one task per
  connection, register handlers by method name, bounded in-flight
  connections, idle and total request deadlines, bounded writes, TCP or
  Unix-domain sockets (Unix only), `tcp://` hostnames, a 10 MiB request cap,
  and a per-request OKEY/ERROR access log (default on).
- **Client** (`YarClient`): blocking API with `connect()` / `call()` /
  `close()`, persistent connections, a 1-second default timeout, a 10 MiB
  response cap checked before allocation, `tcp://` or `host:port`,
  connect-timeout that also bounds DNS resolution, poisoned failed
  connections, and `call_raw()` / `build_request_packet()` for pre-packed
  hot paths.
- **Data model** (`Packager`, `Unpackager`, `YarData`): a safe Rust tree for
  the Yar types (null, bool, long, ulong, double, string, array, map), with
  parser limits against malicious nesting, oversized lengths, excessive node
  counts and trailing bytes.
- **Request / response**: the exact `{i,m,p}` / `{i,s,r,e}` envelopes; server
  errors use status 8 and error text is truncated to 1023 bytes.
- **Logger** (`YarLogger`): file/stderr/pipe output with the
  OKEY/DEBUG/NOTICE/WARNING/ERROR levels, including the access-log level.

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

## Example

Complete runnable examples live in the
[`examples/`](https://github.com/lovelock/yar-rs/tree/master/examples) directory:

```sh
cargo run --example cross_server   # in one terminal
cargo run --example cross_client   # in another
```

### Server

```rust,no_run
use yar_rs::{Packager, YarResponse, YarServer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = YarServer::new("0.0.0.0:9501");

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

The suite covers the protocol header, request/response packing, the
packager, wire-format compatibility vectors, client↔server integration over
TCP and Unix sockets, and security regressions for malformed msgpack and
untrusted response sizes.

## Profiler (C vs Rust benchmark)

The benchmark that compares this implementation with the original C project
lives in [`profile/`](https://github.com/lovelock/yar-rs/tree/master/profile)
(not part of the published crate). It runs a k6-style matrix with one
persistent connection per virtual user and reports attempt QPS, **successful
QPS (`ok_qps`)**, ok/fail, p50/p95/p99/p99.9, server CPU share, memory peak
and CFS throttling, plus the load generator's CPU share.

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
