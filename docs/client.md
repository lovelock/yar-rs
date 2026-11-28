# Client

[Back to the guide index](README.md)

`YarClient` is a blocking client, matching the C client's model: one
connection, one in-flight call at a time, one response per call. It can be
used from ordinary threads and, with `spawn_blocking`, from async code.

```rust,no_run
use std::time::Duration;
use yar_rs::{Packager, YarClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = YarClient::connect("tcp://127.0.0.1:9501")?;
    client.set_timeout(Duration::from_secs(1));
    client.set_persistent(true);

    let mut a = Packager::new();
    a.push_long(10)?;
    let mut b = Packager::new();
    b.push_long(20)?;

    let response = client.call("add", vec![a, b])?;
    if response.get_status() == 0 {
        println!("add = {:?}", response.get_response());
    }
    client.close();
    Ok(())
}
```

## Addresses

| Form | Meaning |
|---|---|
| `host:port` | TCP |
| `tcp://host:port` | TCP; the C-style scheme is accepted |
| `/path/to.sock` | Unix domain socket (Unix only; other platforms return an error) |
| `http://` / `https://` | Rejected: Yar is a raw TCP protocol |

## Configuration

| Method | Default | Effect |
|---|---|---|
| `set_timeout(Duration)` | 1 s | Read and write timeout for each call (C's default) |
| `set_max_response_size(bytes)` | 10 MiB | Reject a larger declared response before allocating |
| `set_persistent(bool)` | `false` | Set the persistent flag; reuse the connection across calls |
| `connect_with_timeout(host, d)` | 1 s | Bounds DNS resolution and each TCP connect attempt |

## Call semantics

- The request id is fixed at 1000, like the C client. The client validates
  that the response echoes the same id.
- `call(method, params)` packs each `Packager` into one array and sends it as
  the request's `p` value. An empty `params` sends `p = null`.
- A successful call returns `Ok(YarResponse)`. A server-side failure is a
  response with a non-zero `status` (server errors use 8), not an `Err`.
- Transport errors, timeouts and malformed responses are `Err`. After any
  `Err`, the client is **poisoned**: a late response may still be in the
  socket, so further calls fail until you reconnect.
- A non-persistent client can make exactly one call. The server closes after
  the response; create a new client for the next call or enable persistence.

## Pre-packed requests

`call` rebuilds the request envelope each time. When the same request is sent
many times (a benchmark, a batch job), build the wire packet once and reuse
it:

```rust,no_run
use yar_rs::client::build_request_packet;
use yar_rs::{Packager, YarClient, YarResponse, PACKAGER_NAME};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut value = Packager::new();
    value.push_string(b"payload")?;
    let mut args = Packager::start_array(1)?;
    args.push_packager(&value)?;

    // 82-byte header + "MSGPACK\0" + msgpack request.
    let packet = build_request_packet("echo", Some(args), true)?;

    let mut client = YarClient::connect("127.0.0.1:9501")?;
    client.set_persistent(true);

    // Raw response body: 8-byte packager name + msgpack.
    let raw = client.call_raw(&packet)?;
    let response = YarResponse::unpack(&raw, PACKAGER_NAME.len())?;
    assert_eq!(response.get_status(), 0);
    Ok(())
}
```

`call_raw` returns the response body unchanged (including the packager
name); parse it with `YarResponse::unpack(&body, 8)`. It applies the same
response size cap and poisoning rules as `call`.

## In async code

The client is blocking, so keep it off the async executor:

```rust,ignore
let response = tokio::task::spawn_blocking(move || {
    let mut client = YarClient::connect("127.0.0.1:9501")?;
    client.call("echo", Vec::new())
})
.await??;
```

Each connection carries one call at a time; there is no request-id
multiplexing. Use one client per task when you need concurrency.
