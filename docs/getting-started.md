# 快速上手

> 本篇文章的示例代码在 `tests/doc_examples.rs::doc_getting_started_minimal` 中有一对一的编译校验。

## 前置要求

- Rust 稳定版工具链（MSRV 1.85）
- 服务端基于 [Tokio](https://tokio.rs)：除了 `yar-rs`，还需要引入 Tokio 运行时

添加依赖：

```toml
[dependencies]
yar-rs = "0.1"

# 服务端是 async 的，需要一个执行器（推荐与 yar-rs 一致的 Tokio）
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net"] }
```

## 30 秒上手：一个完整的最小 RPC

下面是一个"客户端 → 服务端调用 echo"的完整可运行程序。
服务端注册了一个 `echo` 方法并返回字符串 `"echo:ok"`，客户端发起调用并打印返回值。

```rust
use std::thread;
use std::time::Duration;
use yar_rs::{Packager, YarClient, YarRequest, YarResponse, YarServer};

fn main() {
    // ① 服务端：注册 handler 并在 Tokio runtime 上启动
    //    （真实项目中通常是 #[tokio::main] async fn main）
    thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // 监听地址：必须包含端口，如 "127.0.0.1:9501"
            let server = YarServer::new("127.0.0.1:9501");

            // 注册方法名 -> 处理函数。签名要求：
            //   Fn(&YarRequest) -> YarResponse （同步闭包）
            server
                .register_handler("echo", |req: &YarRequest| -> YarResponse {
                    let mut pk = Packager::new();
                    pk.push_string(b"echo:ok").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;

            // 阻塞当前任务直到进程退出
            let _ = server.run().await;
        });
    });

    // 等待服务端真正监听（真实项目可由端口探测替代）
    thread::sleep(Duration::from_millis(300));

    // ② 客户端：发起调用
    let mut client = YarClient::connect("127.0.0.1:9501")
        .expect("连接失败：请检查地址与端口");

    let resp = client.call("echo", vec![]).expect("调用失败");

    // ③ 判断结果：status == 0 才是成功
    if resp.get_status() == 0 {
        let ret = resp.get_response().unwrap(); // YarData
        println!("echo => {}", ret.as_str().unwrap()); // 输出: echo:ok
    } else {
        eprintln!("服务端返回错误: {:?}", resp.get_error());
    }

    client.close();
}
```

要点回顾：

| 步骤 | 关键 API | 注意事项 |
|---|---|---|
| 创建服务端 | `YarServer::new("host:port")` | 支持 `host:port`、`tcp://host:port`、`/path/to.sock`（Unix）；不支持 `http(s)://` |
| 注册方法 | `server.register_handler("名", 闭包).await` | 同名注册会**静默覆盖**旧方法 |
| 启动 | `server.run().await` | 不会返回，直到出错 |
| 连接 | `YarClient::connect("host:port")` | 支持 `tcp://host:port` 与 Unix socket；不支持 `http(s)://` |
| 调用 | `client.call("方法", 参数列表)` | 返回 `YarResult<YarResponse>` |
| 读结果 | `resp.get_status()` / `.get_response()` / `.get_error()` | **status == 0 才算成功** |
| 释放 | `client.close()` | 消费 client 并关闭连接 |

## 运行仓库自带的跨端 demo

仓库自带一对示例服务与客户端（与上面的代码等价，并支持 `-h/-n/-p` 命令行参数）：

```bash
# 终端 1：启动服务（监听 127.0.0.1:9501）
cargo run --example cross_server

# 终端 2：客户端调用
cargo run --example cross_client -- -h 127.0.0.1:9501 -n 2
# 期望输出：
# [round 1/2] [CALL echo] => echo:ok
# [round 1/2] [CALL add 10+20] => 30
# [round 1/2] [CALL greet "Yar"] => Hello, Yar!
# ...
```

查看全部测试与文档示例：

```bash
cargo test
```

## 下一步

- 编写更复杂的 handler → [服务端指南](server.md)
- 了解参数/返回值的类型读取 → [数据模型](data.md)
- 让 PHP / Java 客户端调用你的 Rust 服务 → [协议与跨语言互通](protocol.md)
