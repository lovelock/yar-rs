# 客户端

[返回文档索引](README.md)

`YarClient` 是阻塞客户端，与 C 客户端模型一致：一条连接、同一时刻一次在途调用、
一次调用一个响应。可在普通线程中使用；在 async 代码里用 `spawn_blocking`。

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

## 地址

| 形式 | 含义 |
|---|---|
| `host:port` | TCP |
| `tcp://host:port` | TCP；接受 C 风格前缀 |
| `/path/to.sock` | Unix domain socket（仅 Unix；其他平台返回错误） |
| `http://` / `https://` | 拒绝：Yar 是原始 TCP 协议 |

## 配置

| 方法 | 默认值 | 作用 |
|---|---|---|
| `set_timeout(Duration)` | 1 秒 | 每次调用的读/写超时（C 默认） |
| `set_max_response_size(bytes)` | 10 MiB | 声明响应超限时在分配前拒绝 |
| `set_persistent(bool)` | `false` | 设置持久标志，复用连接 |
| `connect_with_timeout(host, d)` | 1 秒 | 同时约束 DNS 解析与每次 TCP connect |

## 调用语义

- 请求 id 固定为 1000（与 C 客户端一致），客户端校验响应回显同一个 id。
- `call(method, params)` 把每个 `Packager` 包进一个数组作为请求的 `p`；空
  `params` 发送 `p = null`。
- 成功返回 `Ok(YarResponse)`。服务端失败是 `status != 0` 的响应（服务端自动错误为
  status 8），不是 `Err`。
- 传输、超时、响应格式错误是 `Err`。出现 `Err` 后客户端被 **poison**：socket 里
  可能还有迟到的响应，必须重新 `connect` 才能继续。
- 非持久客户端只能成功调用一次：服务端响应后关闭连接；下一次调用需新建 client
  或启用持久连接。

## 预打包请求

`call` 每次都重新拼装请求 envelope。同一个请求要发送很多次时（压测、批处理），
可以只构造一次 wire packet：

```rust,no_run
use yar_rs::client::build_request_packet;
use yar_rs::{Packager, YarClient, YarResponse, PACKAGER_NAME};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut value = Packager::new();
    value.push_string(b"payload")?;
    let mut args = Packager::start_array(1)?;
    args.push_packager(&value)?;

    // 82 字节头 + "MSGPACK\0" + msgpack 请求。
    let packet = build_request_packet("echo", Some(args), true)?;

    let mut client = YarClient::connect("127.0.0.1:9501")?;
    client.set_persistent(true);

    // 原始响应体：8 字节 packager 名 + msgpack。
    let raw = client.call_raw(&packet)?;
    let response = YarResponse::unpack(&raw, PACKAGER_NAME.len())?;
    assert_eq!(response.get_status(), 0);
    Ok(())
}
```

`call_raw` 原样返回响应体（含 packager 名），用
`YarResponse::unpack(&body, 8)` 解析；它同样执行响应体上限与 poison 规则。

## 在 async 代码中使用

客户端是阻塞的，应放到阻塞线程池：

```rust,ignore
let response = tokio::task::spawn_blocking(move || {
    let mut client = YarClient::connect("127.0.0.1:9501")?;
    client.call("echo", Vec::new())
})
.await??;
```

一条连接同一时刻只有一次调用，没有按请求 id 的多路复用；需要并发时为每个任务
创建独立 client。
