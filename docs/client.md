# 客户端指南

> 本文示例在 `tests/doc_examples.rs` 中有编译校验：
> `doc_client_call_with_params`、`doc_client_call_no_params_is_null`、
> `doc_handler_error_response`、`doc_persistent_connection`、`doc_yar_error_matching`。

## 快速总览

```rust
use std::time::Duration;
use yar_rs::{Packager, YarClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 连接地址：必须 "host:port"（或 "tcp://host:port"）；以 "/" 开头视为 Unix domain socket
    let mut client = YarClient::connect("127.0.0.1:9501")?;

    // 可选配置：超时（默认 1 秒，与 C 一致）/ 持久连接（默认 false）
    client.set_timeout(Duration::from_secs(5));
    client.set_persistent(true);

    // 构造参数：每个参数一个 Packager，call() 会把它们包进一个数组发送
    let mut a = Packager::new();
    a.push_long(10)?;
    let mut b = Packager::new();
    b.push_long(20)?;

    let resp: yar_rs::YarResponse = client.call("add", vec![a, b])?;

    if resp.get_status() == 0 {
        println!("add(10, 20) = {}", resp.get_response().unwrap().as_integer().unwrap());
    } else {
        eprintln!("服务端错误: status={} error={:?}", resp.get_status(), resp.get_error());
    }

    client.close();
    Ok(())
}
```

## 连接方法 `connect(hostname)`

| 传入格式 | 行为 |
|---|---|
| `host:port` | TCP 连接（**必须带端口号**，如 `127.0.0.1:9501`） |
| `tcp://host:port` | 同上，接受 C 风格的前缀 |
| `/path/to.sock` | Unix domain socket（仅 Unix 平台；Windows 上返回错误） |
| `localhost`（无端口） | 返回 `Connection("hostname must include port")` |
| `http://…` / `https://…` | 返回 `Connection("HTTP/HTTPS not supported ...")` |

失败会立即返回 `YarError::Connection`/`YarError::Io`（找不到主机、拒绝连接等）。

## 热路径：预打包请求

每次 `call(method, params)` 都会重新拼装参数数组、请求 envelope 和协议头。
如果同一个请求要发送很多次（压测、批处理），可以先构造一次完整 wire packet，
再用 `call_raw` 复用：

- `yar_rs::client::build_request_packet(method, Option<Packager>, persistent) -> YarResult<Vec<u8>>`
  返回 `[82 字节头][MSGPACK\0][msgpack 请求]`；
- `client.call_raw(&packet) -> YarResult<Vec<u8>>` 发送该 packet，返回原始响应体
  （8 字节 packager 名 + msgpack），可用 `YarResponse::unpack(&body, 8)` 解析。

参数含义与 `call` 相同：`params` 是请求的 `p` 值（服务端通常按数组约定读取）。
`call_raw` 同样遵守失败后连接不可复用的语义和响应体上限。

## 超时与持久连接

```rust
client.set_timeout(Duration::from_secs(3)); // 读/写超时，默认 1 秒（与 C 一致）
client.set_max_response_size(10 * 1024 * 1024); // 响应体上限，默认 10 MiB
client.set_persistent(true);                // 见下文
```

如果需要限制建立连接的时间，使用
`YarClient::connect_with_timeout(host, Duration)`；默认建连超时为 1 秒，同时
覆盖主机名解析（解析在辅助线程上执行，超时后返回错误）和 TCP connect。

### ⚠️ 重要的生命周期语义（与原 C 客户端一致）

- **非持久（默认）**：请求头的 `reserved` 不带持久标志。
  服务端**响应完一个请求就关闭连接**。
  因此一个非持久 client 实例**只能成功 call 一次**；第二次 call 会报
  `Connection("connection closed: expected 82 bytes, got 0")`。
- **持久（`set_persistent(true)`）**：连接进入"保持"状态，可连续多次 `call()`，
  直到主动 `close()`、服务端关闭或超时。

选择建议：

```rust
// 方式一：一次性调用（每次调用都新建 client）
let mut client = YarClient::connect(addr)?;
let resp = client.call("echo", vec![])?;
client.close();

// 方式二：连续多次调用 —— 使用持久连接
let mut client = YarClient::connect(addr)?;
client.set_persistent(true);
let resp1 = client.call("add", vec![a, b])?;
let resp2 = client.call("greet", vec![c])?;
client.close();
```

## `call(method, params)` 详解

**参数规则**（与 C 客户端一致）：

- 每个 `Packager` 是一个完整、独立的 msgpack 值（见 [数据模型](data.md)）；
- 客户端会把所有参数包成一个 msgpack **数组**放进请求体的 `p` 字段；
- `vec![]`（无参数）时 `p` 直接为 null。

```rust
use yar_rs::{Packager, YarClient, YarResponse};

let mut client = YarClient::connect("127.0.0.1:9501")?;
client.set_persistent(true);

// 三个参数：add(1, 2, 3)
let mut args: Vec<Packager> = Vec::new();
for x in [1, 2, 3] {
    let mut pk = Packager::new();
    pk.push_long(x)?;
    args.push(pk);
}

let resp: YarResponse = client.call("add", args)?;
assert_eq!(resp.get_response().unwrap().as_integer(), Some(6));
client.close();
```

**返回的 `YarResponse`**：

```text
resp.get_status()  -> i64            # 0 = 成功，非 0 = 业务错误
resp.get_response() -> Option<&YarData>  # 返回值 r 字段；出错时为 None
resp.get_error()    -> Option<&str>  # 错误信息 e 字段；成功时为 None（含客户端未注册方法等）
```

客户端会校验响应大小、packager 名与响应 id；失败时返回传输/协议类 `Err`
（连接断开、超时、非法魔数、响应过大、id 不匹配等）。
**业务失败永远体现在 `status`/`error` 上，而不是 `Err`**。

## 错误处理

`call` 返回 `YarResult<YarResponse>`，传输层错误枚举见
[`YarError`](errors-logging.md)：

```rust
use yar_rs::{YarClient, YarError};

match YarClient::connect("http://example.com:8080") {
    Ok(mut client) => { let _ = client; }
    Err(YarError::Connection(msg)) => eprintln!("连接问题: {}", msg),
    Err(YarError::Timeout) => eprintln!("超时"),
    Err(YarError::InvalidMagic) => eprintln!("对端不是 Yar 服务"),
    Err(e) => eprintln!("其他错误: {}", e),
}
```

## 在 async 代码中使用

`YarClient::call` 是**阻塞 IO**（与 C 客户端一致的同步 API）。
在 Tokio 异步函数里请勿直接调用（会阻塞工作线程），应使用 `spawn_blocking`：

```text
// tokio::task::spawn_blocking
let client = YarClient::connect("127.0.0.1:9501")?;
let resp = tokio::task::spawn_blocking(move || {
    let mut client = client;
    client.set_persistent(true);
    client.call("echo", vec![])
})
.await??;
```

## 相关示例

- 完整命令行客户端：[`examples/cross_client.rs`](../examples/cross_client.rs)
  （支持 `-h 地址`、`-n 轮数`、`-p 持久连接`）
