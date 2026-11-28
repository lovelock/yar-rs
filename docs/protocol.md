# Protocol and Interoperability

[Back to the guide index](README.md)

`yar-rs` speaks the classic Yar wire protocol: an 82-byte header, an 8-byte
packager name, and a msgpack body. The protocol is defined by the original C
project ([laruence/yar-c](https://github.com/laruence/yar-c)); this page
describes the Rust representation and the points that matter when
interoperating.

## Frame

```text
+--------------------------- 82-byte header ---------------------------+
| id u32 | version u16 | magic u32 | flags u32 | provider [u8;32]      |
| token [u8;32]                                                       |
| body_len u32                                                       |
+--------------------------------------------------------------------+
| packager name: 8 bytes, "MSGPACK\0"                                 |
+--------------------------------------------------------------------+
| msgpack body (request or response envelope)                         |
+--------------------------------------------------------------------+
```

`body_len` counts the packager name and the msgpack body. Numeric header
fields are big-endian on the wire. `version` is 0 and the magic is
`0x80DFEC60`.

`YarHeader` stores numeric fields in **host order**; use
`YarHeader::to_bytes()` and `YarHeader::from_bytes()` to convert. Provider
and token copy at most 31 bytes and leave a NUL terminator, matching C's
`strncpy` on the 32-byte fields.

## Flags

| Bit | Constant | Meaning |
|---|---|---|
| `0x1` | `PROTOCOL_PERSISTENT` | The client wants the connection kept open after the response |
| `0x2` | `PROTOCOL_PING` | Reserved; not implemented by either server |
| `0x4` | `PROTOCOL_LIST` | Reserved; not implemented by either server |

A server answers a non-persistent request and closes the connection. A
persistent request loops for the next frame.

## Request envelope

A msgpack map with three keys:

```text
{
  "i": <ulong>,        request id
  "m": <string>,       method name
  "p": <any or null>   parameters
}
```

`YarClient::call` wraps its parameters in an array, so a handler usually sees
`p = [arg0, arg1, ...]`. The server echoes the request id in the response.

## Response envelope

```text
{
  "i": <ulong>,         request id (echoed)
  "s": <int>,           status: 0 = success, non-zero = error
  "r": <any or null>,   return value
  "e": <string or null> error text
}
```

Server-generated errors use status **8** (`YAR_ERROR` in the C server):
oversized body, malformed request, unknown method, unsupported packager.
Error text is truncated to 1023 bytes (1020 + `...`), matching C's
1024-byte buffer. A handler may return any status for business errors.

## Sizes and limits

| Item | Value |
|---|---|
| Header | 82 bytes |
| Packager name | 8 bytes, `MSGPACK\0` |
| Request body cap | 10 MiB by default (`set_max_request_size`) |
| Response body cap | 10 MiB on the client (`set_max_response_size`) |
| Parser depth / nodes | 128 / 262,144 |
| In-flight connections | 1024 by default (`set_max_inflight`) |

The request cap is checked before the body is read. The client checks the
response cap before allocating. Both are separate from the msgpack parser
limits described in [Data model](data.md).

## Interoperability

Because the frame and envelopes match the C implementation, the following
work in either direction:

- a `yar-rs` server with C, PHP, Java or Rust clients;
- a `yar-rs` client with C, PHP, Java or Rust servers.

The packager name must be `MSGPACK`. The optional JSON packager added to
some C builds is not implemented; a JSON-tagged request receives the same
error response the C build returns when cJSON is absent. See
[FAQ](faq.md).

### Differences from the C reference

| Item | yar-rs | C reference |
|---|---|---|
| Server concurrency | one Tokio task per connection, single process | pre-fork event-loop workers |
| Default read timeout | 3 s idle + 30 s total | 3 s per event |
| Write timeout | 5 s | server timeout |
| Server error status | 8 | 8 |
| Error text truncation | 1023 bytes | 1023 bytes |
| `tcp://` hostnames | accepted | accepted |
| Unix sockets | server and client (Unix only) | supported |
| Access log | OKEY/ERROR per request, default on | OKEY/ERROR per request |
| Optional JSON packager | not implemented | optional |
| Daemon/pre-fork/uid-gid | not implemented | supported |
| PING/LIST flags | constants only | constants only |

The C sources and its README remain the authoritative description of the
wire format and the PHP client behavior.

## Provider and token

The client sends its name in `provider` (`Yar(Rust)-0.1.0`); the server
writes its own name in responses. `token` is not used by the Rust
implementation and is sent as zeroes. Neither field affects routing or
authentication.
