# Errors and Logging

[Back to the guide index](README.md)

## Errors

Client and low-level APIs return `YarResult<T> = Result<T, YarError>`.

| Variant | When |
|---|---|
| `Protocol(String)` | Bytes do not follow the protocol (truncated frame, not a map, missing field, too large) |
| `Pack(String)` | msgpack encoding/decoding failed, including parser limits |
| `Io(std::io::Error)` | Socket read/write failure |
| `Connection(String)` | Connect failed, peer closed, or a poisoned client was reused |
| `Timeout` | A socket or server deadline elapsed |
| `InvalidMagic` | The header magic does not match |
| `UnsupportedPackager(String)` | The peer used a packager other than `MSGPACK` |
| `MethodNotFound(String)` | The server has no handler for the method (returned as a status-8 response) |

**Business errors are not `Err`.** A server-side failure is a successful
call whose `YarResponse::get_status()` is non-zero:

```rust,ignore
let response = client.call("divide", vec![a, b])?; // transport/decode only
if response.get_status() == 0 {
    let value = response.get_response();
} else {
    eprintln!("status {}: {:?}", response.get_status(), response.get_error());
}
```

Server-generated errors use status 8 and their text is truncated to 1023
bytes. Handler errors may use any status; `YarResponse::error(id, code,
message)` truncates the message the same way.

After a transport, timeout or decode error, the client poisons the
connection: a late response may still be in the socket. Reconnect with
`YarClient::connect` before the next call.

## Logger

`YarLogger` writes lines in this format:

```text
{hostname} [{YYYY-MM-DD HH:MM:SS}] {LEVEL}: {message}
```

Levels (values match the C logger):

| Level | Value | Notes |
|---|---:|---|
| `LogLevel::Ok` | 0 | Access log, printed as `OKEY`; never filtered |
| `LogLevel::Debug` | 1 | Protocol/trace details |
| `LogLevel::Notice` | 2 | Default server threshold |
| `LogLevel::Warning` | 4 | Warnings |
| `LogLevel::Error` | 8 | Errors |

A message below the configured threshold is suppressed. `OKEY` is always
emitted when an access log is enabled.

The server initializes the process-global logger on `run()` from
`set_log_file` and `set_log_level`:

```rust,ignore
server.set_log_file("/var/log/yar-rs.log"); // or "|command"; default stderr
server.set_log_level(LogLevel::Notice);
server.set_access_log(true);
```

You can also initialize a global logger yourself with
`logger::init_global(path, level)`; it can only succeed once per process. A
standalone `YarLogger::init` instance is available for local use.

### Access log

When `set_access_log(true)` (default), each completed request writes one
line, matching the C server's fields:

```text
<id> <peer> "<method>" "<provider>" <bytes_sent> <elapsed_us>     # success
<id> <peer> "<method>" <error> "<provider>" <bytes_sent> -        # status != 0
```

CR/LF in the method, error or provider are escaped. Disable the access log
for benchmarks or high-volume deployments:

```rust,ignore
server.set_access_log(false);
// or, for the bundled binary:
// yar-server-rust --no-access-log
```

The `yar-server-rust` binary also accepts `--log-file` and `--log-level`.
