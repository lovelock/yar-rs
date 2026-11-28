# 快速上手

[返回文档索引](README.md)

## 前置要求

- Rust 1.85 或更新。
- 服务端基于 [Tokio](https://tokio.rs)，除 `yar-rs` 外还需引入所需的 Tokio features。
- 客户端是阻塞 API，不依赖 Tokio。

添加依赖：

```toml
[dependencies]
yar-rs = "0.1"

[dependencies.tokio]
version = "1"
features = ["rt-multi-thread", "macros", "net"]
```

## 最小服务端

```rust,no_run
use yar_rs::{Packager, YarResponse, YarServer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = YarServer::new("0.0.0.0:9501");

    // handler 是同步闭包：Fn(&YarRequest) -> YarResponse。
    server
        .register_handler("echo", |req| {
            let mut pk = Packager::new();
            pk.push_string(b"echo:ok").unwrap();
            YarResponse::success(req.id(), pk)
        })
        .await;

    // 一直运行到进程退出（或 accept/初始化失败）。
    server.run().await?;
    Ok(())
}
```

`YarServer::new` 接受 `host:port`、`tcp://host:port`，或在 Unix 上以 `/` 开头的
socket 路径。默认值偏安全：10 MiB 请求上限、1024 在途连接、3 秒 idle 读超时、
30 秒请求总 deadline、5 秒写超时、每请求访问日志。全部选项见[服务端](server.md)。

## 最小客户端

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
        eprintln!("服务端错误 {}: {:?}", response.get_status(), response.get_error());
    }

    client.close();
    Ok(())
}
```

业务错误**不是** `Err`，而是 `status` 非 0 的成功响应（服务端自动生成的错误用
status 8）。传输失败、超时、解码错误才是 `Err`。出现 `Err` 后客户端连接会被
poison，必须重新 `connect`；详见[客户端](client.md)。

客户端是阻塞的。在 async 程序中请放到 `tokio::task::spawn_blocking`，或每个
client 用一个独立线程。

## 运行仓库示例

```sh
cargo run --example cross_server   # 终端 1
cargo run --example cross_client   # 终端 2
```

客户端会执行几轮 `echo`、`add`、`greet` 调用。

## 运行测试

```sh
cargo test
cargo test --all-features   # 同时构建 profiler 工具
```

## 下一步

- [服务端](server.md) — handler、生命周期、限制、访问日志。
- [客户端](client.md) — 持久连接、超时、预打包请求。
- [数据模型](data.md) — 参数与返回值。
- [协议](protocol.md) — 线上格式与 C/PHP/Java 互通。
