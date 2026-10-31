# yar-rs

Rust implementation of the [Yar](https://github.com/laruence/yar) RPC framework.

(see also: [Yar PHP framework](https://github.com/laruence/yar), [Yar Java framework](https://github.com/weibocom/yar-java))

`yar-rs` is a faithful, idiomatic **async Rust** rewrite of the original
Yar C framework: it speaks the exact same wire protocol, so a `yar-rs`
server can serve requests from Yar clients written in PHP, Java, C, or
Rust — and vice versa.

## Features

- **Wire-compatible** with the classic Yar protocol: 82-byte big-endian
  header (`YAR_PROTOCOL_VERSION 0`, magic `YAR`), followed by a packager
  name and a msgpack-encoded body.
- **Client** (`YarClient`): blocking API with `connect()` / `call()` /
  `close()`, persistent-connection and read-timeout options.
- **Server** (`YarServer`): Tokio-based async server, register handlers by
  method name, standalone or multi-worker mode.
- **Pack / Unpack** (`Packager`, `YarData`): msgpack based pack/unpack with
  the same data model as the C version (null, bool, long, ulong, double,
  string, array, map).
- **Request / Response**: exact binary format of `yar_request` /
  `yar_response` (id, method, parameters / status, error, retval).
- **Logger** (`YarLogger`): file logger with the same log levels as the C
  version.

## Install

```toml
[dependencies]
yar-rs = "0.1"
```

Build from source:

```
cargo build --release
```

## Example

Complete runnable examples live in the [`examples/`](examples/) directory:

```
cargo run --example cross_server   # in one terminal
cargo run --example cross_client   # in another terminal
```

### Server

```rust
use yar_rs::{YarServer, YarResponse, Packager};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = YarServer::new("127.0.0.1:9501");

    server
        .register_handler("echo", |req| async move {
            let mut pk = Packager::new();
            pk.push_string(b"echo:ok")?;
            Ok(YarResponse::success(req.id(), pk))
        })
        .await;

    server.run().await?;
    Ok(())
}
```

> Note: the handler signature above is illustrative; see
> `examples/cross_server.rs` and the tests for exact usage.

### Client

```rust
use yar_rs::YarClient;

let mut client = YarClient::connect("127.0.0.1:9501")?;
let response = client.call("add", params)?;
if response.get_status() == 0 {
    println!("result = {:?}", response.get_response());
}
client.close();
```

## Tests

```
cargo test
```

The test suite (161 tests) includes:

- Unit tests for protocol header render/parse, request/response
  pack/unpack, and the packager/unpacker.
- Wire-format compatibility tests: byte streams recorded from the original
  C implementation are parsed by the Rust code, and Rust-produced packets
  are verified to match the C wire layout.
- Full client↔server integration tests over real TCP connections.

## License

Apache-2.0 (see [LICENSE](LICENSE)).
