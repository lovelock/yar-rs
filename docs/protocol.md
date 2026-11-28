# 协议与跨语言互通

> 本文底层示例在 `tests/doc_examples.rs` 中有编译校验：
> `doc_lowlevel_request_response_roundtrip`、`doc_yar_header_wire`。

本文完整描述 yar-rs 与原 C 实现共享的线上协议，帮助你：

- 抓包调试；
- 自定义传输层（例如在其他环境中重用 yar 的编解码）；
- 与 PHP / Java / C 的 Yar 服务互操作。

## 线上包结构

每个请求/响应都是三段连续字节：

```text
┌────────────────────────┐
│ 协议头  82 字节 (大端)  │
├────────────────────────┤
│ packager 名  8 字节     │  b"MSGPACK\0"
├────────────────────────┤
│ msgpack 请求/响应体     │  body_len 指明长度
└────────────────────────┘
```

- 头部中的 `body_len` 只统计 **packager 名 + msgpack 体** 的长度（不含头本身）。
- 批量模式（`PROTOCOL_LIST` 等）目前仅保留协议常量，未实现。

## 协议头 `YarHeader`（82 字节，网络序 = 大端）

| 偏移 | 大小 | 字段 | 说明 |
|---|---:|---|---|
| 0 | 4 | `id: u32` | 请求 ID。Rust 客户端固定使用 `1000`（与 C 客户端相同），服务端原样回填 |
| 4 | 2 | `version: u16` | 协议版本，恒为 0 |
| 6 | 4 | `magic_num: u32` | 魔数 `0x80DFEC60`，解析时必须校验 |
| 10 | 4 | `reserved: u32` | 标志位：`0x1` 持久连接 `PROTOCOL_PERSISTENT`，`0x2` ping，`0x4` list |
| 14 | 32 | `provider: [u8;32]` | 客户端名称。render() 最多写入 31 字节并保留结尾 NUL（Rust 客户端：`"Yar(Rust)-0.1.0"`） |
| 46 | 32 | `token: [u8;32]` | 鉴权 token，未使用，全零 |
| 78 | 4 | `body_len: u32` | body 长度 = 8 + msgpack 体长 |

注意：`YarHeader` 的数值字段是**主机字节序**，可以直接按字段名读取/写入。
与网络交互时使用 `to_bytes()` 生成大端线上字节、`from_bytes()` 解析并校验魔数。

```rust
use yar_rs::{YarHeader, PROTOCOL_PERSISTENT, HEADER_SIZE};

let mut header = YarHeader::new();
// 参数：id, provider(最多 31 字节), token(最多 31 字节), body_len, reserved(标志位)
// render() 写入的是主机字节序字段，不需要再调用 parse()。
header.render(1000, Some("Yar(Rust)-0.1.0"), None, 128, PROTOCOL_PERSISTENT);

// to_bytes() 生成线上大端字节，可以直接 send 给 server / client
let wire = header.to_bytes();
assert_eq!(wire.len(), HEADER_SIZE); // 82

// 对端解析（校验魔数 0x80DFEC60，不匹配返回 Err）
let parsed = YarHeader::from_bytes(&wire).unwrap();
assert_eq!(parsed.id, 1000);
assert_eq!(parsed.body_len, 128);
let wire_id = u32::from_be_bytes(wire[0..4].try_into().unwrap());
assert_eq!(wire_id, 1000);
```

## msgpack 请求体

请求体是一个 msgpack map（3 个 key）：

```text
{
  "i": <unsigned int>,     请求 ID（回显）
  "m": <string>,           方法名
  "p": <array> 或 <null>,  参数：无参时为 null；有参时为由各参数 msgpack 值组成的数组
}
```

示例：调用 `add(10, 20)` 时 msgpack 体（16 进制语义）：

```text
map(i=1000, m="add", p=[10, 20])
```

## msgpack 响应体

响应体同样是 msgpack map（4 个 key）：

```text
{
  "i": <unsigned int>,    请求 ID（原样返回）
  "s": <int>,             状态码：0 = 成功，非 0 = 错误；服务端生成的错误统一为 8（C 的 YAR_ERROR）
  "r": <any> 或 <null>,   返回值；出错时无
  "e": <string> 或 <null> 错误信息；成功时为 null
}
```

例：服务端返回 `status=10000, e="division by zero"` →
`{"i":1000, "s":10000, "r":null, "e":"division by zero"}`。

## 底层 API：直接使用 request / response

不通过 client/server，也可以直接以协议格式编解码：

```rust
// 请求体是 msgpack map：{"i": id, "m": 方法名, "p": 参数}
let mut params = Packager::start_array(2).unwrap();
params.push_long(42).unwrap();
params.push_string(b"world").unwrap();

let mut req = YarRequest::new(1000, "echo");
req.set_parameters(params);

let req_bytes = req.pack(0).unwrap();          // pack(前导字节数)：可在前面预留 header+packager 名空间
let parsed_req = YarRequest::unpack(&req_bytes, 0).unwrap();
assert_eq!(parsed_req.id(), 1000);
assert_eq!(parsed_req.method(), "echo");
assert_eq!(parsed_req.get_parameters().unwrap().as_array().unwrap().len(), 2);

// 响应体是 msgpack map：{"i": id, "s": 状态, "r": 返回值, "e": 错误}
let mut retval = Packager::new();
retval.push_string(b"echo:ok").unwrap();
let resp = YarResponse::success(1000, retval);

let resp_bytes = resp.pack(0).unwrap();
let parsed_resp = YarResponse::unpack(&resp_bytes, 0).unwrap();
assert_eq!(parsed_resp.get_status(), 0);
assert_eq!(parsed_resp.get_response().unwrap().as_str(), Some("echo:ok"));
```

`pack(extra_bytes)` 的语义：前 `extra_bytes` 字节零填充，紧跟 msgpack 体——
配合协议头/包名声（82 + 8 = 90）可在同一块 buffer 上就地组装线上数据，与 C 版 API 形状一致。

## 持久连接语义

- 客户端在**请求头**的 `reserved` 置 `0x1`，表示"我这是保持的连接"；
- 服务端响应后**不关闭**连接，继续等待下一个请求；
- 服务端若收到一条**不带**该标志的请求，会在本次响应后关闭连接；
- 服务端每个请求的读取超时（`set_read_timeout`，默认 3 秒）超时后也会关闭。

## 跨语言互通

### PHP 客户端 → yar-rs 服务端

PHP 客户端 (iyar / pecl yar) 使用相同的转换层，只要 msgpack 扩展可用即可：

```php
// PHP
$client = new Yar_Client("tcp://127.0.0.1:9501");
var_dump( $client->add(10, 20) );   // 30
var_dump( $client->greet("Yar") );  // "Hello, Yar!"
```

> 注意：PHP Yar 客户端可能要求 `"MSGPACK"` 打包器（yar.packager 配置为 msgpack）。

### C / Java 客户端 → yar-rs 服务端

C 客户端：

```c
// 原始 C 版本的用法
yar_client *client = yar_client_init("tcp://127.0.0.1:9501");
yar_response *resp = client->call(client, "add", 2, args);
```

Java（weibocom/yar-java）同样按上述 msgpack 请求帧实现服务端只需正确解析即可。

### yar-rs 客户端 → 其他 Yar 服务

- 可以连接**任何实现了相同 TCP 协议**的 Yar 服务（C 版 yar_server 等）；
- **不能**连 PHP 实现的 Yar **服务**：PHP 的 YAR server 基于 HTTP 传输，而 yar-rs 仅支持原始 TCP —— 这也是原 C 版的限制（C 请求 PHP 服务同样未开放）。
