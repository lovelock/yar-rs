# Getting Started

[Back to the guide index](README.md)

## Prerequisites

- Rust 1.85 or newer.
- The server runs on [Tokio](https://tokio.rs), so add the runtime features
  you need in addition to `yar-rs`.
- The client is blocking and does not require Tokio.

Add the crate:

```toml
[dependencies]
yar-rs = "0.1"

[dependencies.tokio]
version = "1"
features = ["rt-multi-thread", "macros", "net"]
```

## A minimal server

```rust,no_run
use yar_rs::{Packager, YarResponse, YarServer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = YarServer::new("0.0.0.0:9501");

    // Handlers are synchronous closures: Fn(&YarRequest) -> YarResponse.
    server
        .register_handler("echo", |req| {
            let mut pk = Packager::new();
            pk.push_string(b"echo:ok").unwrap();
            YarResponse::success(req.id(), pk)
        })
        .await;

    // Runs until the process exits (or an accept/setup error occurs).
    server.run().await?;
    Ok(())
}
```

`YarServer::new` accepts `host:port`, `tcp://host:port`, or (on Unix) a
socket path beginning with `/`. Defaults are chosen for safety: 10 MiB
request cap, 1024 in-flight connections, 3 s idle read timeout, 30 s total
request deadline, 5 s write timeout, and a per-request access log. See
[Server](server.md) for every option.

## A minimal client

```rust,no_run
use yar_rs::{Packager, YarClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = YarClient::connect("127.0.0.1:9501")?;

    let mut name = Packager::new();
    name.push_string(b"Yar")?;

    let response = client.call("greet", vec![name])?;
    if response.get_status() == 0 {
        println!("{:?}", response.get_response());
    } else {
        eprintln!("server error {}: {:?}", response.get_status(), response.get_error());
    }

    client.close();
    Ok(())
}
```

A business error is **not** an `Err`: it is a successful response whose
`status` is non-zero (server-generated errors use status 8). Transport
failures, timeouts and decode errors are `Err`. After any `Err`, the client
is poisoned and must be reconnected; see [Client](client.md).

The client is blocking. Inside an async program, run calls on
`tokio::task::spawn_blocking`, or use one thread per client.

## Run the repository examples

```sh
cargo run --example cross_server   # terminal 1
cargo run --example cross_client   # terminal 2
```

The client prints a couple of rounds of `echo`, `add` and `greet` calls.

## Run the tests

```sh
cargo test
cargo test --all-features   # also builds the profiler tooling
```

## Next

- [Server](server.md) — handlers, lifecycle, limits, access log.
- [Client](client.md) — persistent connections, timeouts, pre-packed
  requests.
- [Data model](data.md) — parameters and return values.
- [Protocol](protocol.md) — the wire format and C/PHP/Java interop.
