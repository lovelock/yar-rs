# Server

[Back to the guide index](README.md)

`YarServer` is an async Tokio service. It owns one listener, accepts
connections, and spawns a task per connection. A connection task reads
requests sequentially, calls a registered handler, writes the response, and
loops while the client's persistent flag is set. There is no pre-fork, no
daemonization, and no worker process to tune: Tokio's worker threads provide
the parallelism.

```rust,no_run
use std::time::Duration;
use yar_rs::logger::LogLevel;
use yar_rs::{Packager, YarResponse, YarServer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = YarServer::new("0.0.0.0:9501");
    server.set_read_timeout(Duration::from_secs(3));
    server.set_max_inflight(1024);
    server.set_log_file("/var/log/yar-rs.log");
    server.set_log_level(LogLevel::Notice);
    server.set_access_log(true);
    server.set_pid_file("/run/yar-rs.pid");

    server
        .register_handler("add", |req| {
            let arr = req.get_parameters().and_then(|p| p.as_array()).unwrap_or(&[]);
            let sum: i64 = arr.iter().filter_map(|v| v.as_integer()).sum();
            let mut pk = Packager::new();
            pk.push_long(sum).unwrap();
            YarResponse::success(req.id(), pk)
        })
        .await;

    server.run().await?;
    Ok(())
}
```

## Handlers

A handler is a synchronous closure `Fn(&YarRequest) -> YarResponse` that is
stored by method name. Registering the same name again replaces the old
handler. The response id is always overwritten with the request id before
sending, so a handler may leave it at 0.

A handler executes on the connection task, which runs on a Tokio worker
thread. Blocking or CPU-heavy work should be moved off that thread (for
example with `tokio::task::block_in_place` on a multi-thread runtime, or with
your own thread pool) so other connections are not stalled. Handlers cannot
`await`; the signature is deliberately synchronous, like the C callbacks.

Business failures are returned as responses, not errors:

```rust,ignore
let mut resp = YarResponse::error(req.id(), 1000, "division by zero");
```

Use status `0` for success. Server-generated errors (oversized body,
malformed request, unknown method, unsupported packager) use status **8**,
matching the C server, and their text is truncated to 1023 bytes.

## Configuration

| Method | Default | Effect |
|---|---|---|
| `set_max_request_size(bytes)` | 10 MiB | A larger declared body is rejected before the body is read |
| `set_max_inflight(n)` | 1024 | Connection-level gate; a permit is held for the connection's lifetime; `0` disables the gate |
| `set_read_timeout(Duration)` | 3 s | Idle timeout applied to each socket read; a slow trickle keeps it alive |
| `set_request_timeout(Option<Duration>)` | 30 s | Total deadline for receiving one header+body; `None` disables it |
| `set_write_timeout(Duration)` | 5 s | A peer that stops reading cannot pin the task forever |
| `set_log_file(path)` | stderr | Log destination (`stderr`, a file path, or a `|command` pipe) |
| `set_log_level(LogLevel)` | `Notice` | Minimum level; OKEY is always emitted |
| `set_access_log(bool)` | `true` | Per-request OKEY/ERROR line |
| `set_pid_file(path)` | none | Write the process id after the listener binds |

The listener address comes from `YarServer::new`:

- `host:port` — TCP.
- `tcp://host:port` — same, C-style scheme accepted.
- `/path/to.sock` — Unix domain socket on Unix platforms; other platforms
  return an unsupported error.

## Connection lifecycle

1. `read_timeout` bounds each individual read; `request_timeout` bounds the
   whole header+body exchange. Both are reset for the next request.
2. The body is read only after the declared length passes
   `max_request_size`.
3. The packager tag must be `MSGPACK`; otherwise the server sends an error
   response (status 8) and then closes a non-persistent connection.
4. The request envelope is decoded; malformed input receives an error
   response.
5. The handler is looked up by method name. An unknown method receives
   status 8 with `"Method not found: ..."`.
6. The response is written under `write_timeout`; its id echoes the request
   id and its provider is the server name.
7. A non-persistent request closes the connection. A persistent request loops
   to step 1.

## Access log

With `set_access_log(true)` (the default), every completed request produces
one line, matching the C server's fields:

```text
<id> <peer> "<method>" "<provider>" <bytes_sent> <elapsed_us>     # success
<id> <peer> "<method>" <error> "<provider>" <bytes_sent> -        # status != 0
```

CR/LF in the method, error or provider are escaped so a peer cannot forge an
extra log line. Disable it in benchmarks and high-throughput deployments
with `set_access_log(false)` or the binary's `--no-access-log`.

## Shutdown

`run()` serves until the process exits or setup fails. There is no graceful
drain API: stopping the process closes the listener and drops the connection
tasks. This is a deliberate difference from process-management features in
other servers; use your process supervisor to stop the service.

## Not implemented

- Daemonization, pre-fork workers, worker restart, uid/gid drop.
- The optional JSON packager (a JSON-tagged request receives the same error
  response the C build returns without cJSON).
- `PROTOCOL_PING` / `PROTOCOL_LIST` (constants only, as in the C server).

See [FAQ](faq.md) for the design rationale and the full C-compatibility
table.
