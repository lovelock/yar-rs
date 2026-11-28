# yar-rs User Guide

**English** | [中文](zh-CN/README.md)

`yar-rs` is a Rust implementation of the Yar RPC protocol. It is designed for
Rust rather than as a line-by-line port of the C framework: the server is an
async Tokio service, the client is a blocking API, and the data model is a
safe Rust tree. The wire format stays compatible with the original C
implementation, so a `yar-rs` server can serve Yar clients written in C, PHP,
Java or Rust, and a `yar-rs` client can call them.

The C project remains the reference for the protocol:
[laruence/yar-c](https://github.com/laruence/yar-c) and the PHP extension
[laruence/yar](https://github.com/laruence/yar). This guide describes the
Rust design and API; it is not a translation of the C manual.

## Guides

| Guide | Contents |
|---|---|
| [Getting started](getting-started.md) | Install the crate, run a minimal server and client, run the repository examples |
| [Server](server.md) | `YarServer` design, handlers, connection lifecycle, timeouts, limits, Unix sockets, access log |
| [Client](client.md) | `YarClient` addresses, timeouts, persistence, failure handling, pre-packed requests |
| [Data model](data.md) | `Packager`, `Unpackager`, `YarData`, msgpack types, parser limits |
| [Protocol](protocol.md) | Wire header, request/response envelopes, status codes, interoperability with C/PHP/Java |
| [Errors and logging](errors-logging.md) | `YarError`, status-vs-transport errors, logger levels, access log |
| [FAQ](faq.md) | Design decisions, limitations, common questions |
| [Benchmark](https://github.com/lovelock/yar-rs/blob/master/profile/README.md) | C vs Rust harness, parity rules, report columns and results |

## Design at a glance

- **Server**: a Tokio listener accepts connections; each connection runs as
  its own task, reads requests sequentially, invokes a registered synchronous
  handler, writes the response, and loops while the persistent flag is set.
  There is no pre-fork or daemon mode. A connection gate and idle/total
  deadlines bound resource use.
- **Client**: a blocking API, one in-flight call per connection, matching the
  C client's model. Persistent connections are supported. A failed call
  poisons the connection so a late response cannot be mistaken for the next
  one.
- **Data**: msgpack bytes are parsed into `YarData` and written by
  `Packager`. The parser enforces a maximum depth, a maximum node count,
  input-bounded lengths and no trailing bytes.
- **Defaults** (all configurable): 10 MiB request/response caps, 1024
  in-flight connections, 3 s idle read timeout, 30 s total request deadline,
  5 s write timeout, 1 s client timeout, status 8 for server-generated
  errors, and a per-request OKEY access log.

## Reading order

1. First time: [Getting started](getting-started.md), then the
   [Client](client.md) section on persistent connections.
2. Writing handlers: [Server](server.md) and [Data model](data.md).
3. Connecting existing Yar services: [Protocol](protocol.md).
4. Troubleshooting: [Errors and logging](errors-logging.md) and the
   [FAQ](faq.md).

## Relationship to the C reference

| Aspect | yar-rs | C reference |
|---|---|---|
| Server concurrency | Tokio tasks, one per connection, single process | pre-fork event-loop workers |
| Client | blocking, one in-flight call per connection | blocking, same model |
| Wire format | identical 82-byte header + `MSGPACK\0` + msgpack | reference implementation |
| Server error status | 8 (`YAR_ERROR`) | 8 |
| Error text limit | 1023 bytes | 1023 bytes |
| Request cap | 10 MiB | 10 MiB |
| Read timeout | 3 s idle + 30 s total (configurable) | 3 s per event (configurable) |
| Access log | OKEY/ERROR per request, default on | OKEY/ERROR per request |
| Optional JSON packager | not implemented | optional at build time |
| Daemon/pre-fork/uid-gid | not implemented (deliberate) | supported |

The compatibility details live in [Protocol](protocol.md) and the
[FAQ](faq.md). The original C sources and documentation are the authority for
the wire format.

## Repository layout

```
src/        library: protocol, pack, request, response, client, server, logger
examples/   cross_server / cross_client
tests/      unit, integration, security, doc-example checks
profile/    C-vs-Rust benchmark harness and reports (not published)
docs/       this guide (English at the root, Chinese in docs/zh-CN)
```
