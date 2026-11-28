# FAQ

[Back to the guide index](README.md)

## What is the relationship to the original Yar project?

`yar-rs` implements the Yar RPC protocol in Rust. It is an independent
project, not a fork of the C code or its documentation. The C implementation
([laruence/yar-c](https://github.com/laruence/yar-c)) and the PHP extension
([laruence/yar](https://github.com/laruence/yar)) remain the reference for
the wire format and for existing deployments. Because the frame and
envelopes match, a `yar-rs` server and a C/PHP/Java client (or the reverse)
can talk to each other.

## Why does the server not look like the C server?

The C server uses pre-forked event-loop workers. `yar-rs` uses one process
with a Tokio listener and one task per connection. That gives the same
external behavior (concurrent persistent connections, sequential requests
per connection) with a Rust-native concurrency model. There is no worker
count to tune; Tokio's worker threads follow the process's CPU availability.

## Can I run it as a daemon or pre-fork it?

No. Daemonization, pre-fork workers, worker restart and uid/gid drop are not
implemented. Run it under your process supervisor or service manager. The
PID file option writes the current PID for tooling that expects one.

## How do I stop the server gracefully?

`YarServer::run()` serves until the process exits. There is no drain API:
stopping the process closes the listener and drops the connection tasks.
Use the supervisor's stop signal and let the process exit.

## How many connections can it serve?

The default `max_inflight` is 1024 connections. A permit is held for the
whole connection, so persistent connections count until they close. Set
`set_max_inflight(0)` to disable the gate. Memory and file descriptors are
the real limits; each connection buffers at most one request body.

## Handlers are synchronous. Where do I put slow work?

A handler runs on the connection task's worker thread. Move blocking or
CPU-heavy work off it with `tokio::task::block_in_place` on a multi-thread
runtime, or with your own thread pool. Handlers cannot `await`; this keeps
the API close to the C callback model.

## Why is the client blocking?

It matches the C client and is simple to embed in synchronous programs.
From async code, use `tokio::task::spawn_blocking`. There is one in-flight
call per connection and no request-id multiplexing; use one client per task
for concurrency.

## Why do I get a "poisoned" error after a timeout?

A timeout or transport error can leave a late response in the socket.
Reusing the connection could return that stale response for the next call,
so the client refuses further calls until you reconnect.

## Why does a call return `Ok` with a non-zero status?

Business failures are responses, not transport errors: check
`YarResponse::get_status()`. Server-generated errors use status 8. Transport
and decode failures are `Err`.

## Where do the error status and truncation numbers come from?

They match the C server: server-generated status 8 (`YAR_ERROR`) and error
text truncated to 1023 bytes (1020 + `...`) so the frame fits the C
server's 1024-byte error buffer.

## Is `tcp://` supported? Unix sockets? Windows?

`tcp://host:port` is accepted by the client and server. Unix domain sockets
are supported on Unix for both sides; on other platforms they return an
unsupported error. TCP works on Windows and macOS; the test suite runs on
all three.

## What are the size limits?

10 MiB request body (server) and response body (client), checked before
allocation. The msgpack parser also limits nesting to 128 and the number of
nodes to 262,144, rejects declared lengths larger than the remaining input,
and rejects trailing bytes.

## Are the C and Rust values byte-identical?

For the supported types, yes: same msgpack compact integer encoding, same
envelopes, same header layout. See [Protocol](protocol.md). The provider
name and the client implementation name differ; neither affects routing.

## Why is `4 MiB × 640 VUs` missing from the benchmark?

That cell needs more than 4 GiB of server-side body buffers (640 × 4 MiB)
before any copies, so the 2 GiB server is OOM-killed. The benchmark
documents this and uses `--sizes`/`--vus` for feasible subsets. See
[the benchmark guide](https://github.com/lovelock/yar-rs/blob/master/profile/README.md).

## What is not implemented?

- The optional JSON packager (a JSON-tagged request gets the same error the
  C build without cJSON returns).
- `PROTOCOL_PING` / `PROTOCOL_LIST` (constants only, as in the C server).
- Request-id multiplexing, daemon/pre-fork, graceful drain.

## I am coming from the C API. Where do I start?

Read [Getting started](getting-started.md) and [Client](client.md), then the
small mapping table in the [README](../README.md) for the Rust names of the
C concepts. For the protocol itself, the C project's README is the
authority.
