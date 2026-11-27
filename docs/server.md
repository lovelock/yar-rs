# 服务端指南

> 本文示例在 `tests/doc_examples.rs` 中有编译校验：
> `doc_getting_started_minimal`、`doc_client_call_with_params`、
> `doc_server_map_params_and_nested_retval`、`doc_handler_error_response`、
> `doc_persistent_connection`。

## 快速总览

```rust
use yar_rs::{Packager, YarRequest, YarResponse, YarServer};

#[tokio::main]
async fn main() {
    let mut server = YarServer::new("127.0.0.1:9501");

    // --- 可选配置（必须在 run() 之前设置） ---
    server.set_max_inflight(1024);
    server.set_read_timeout(std::time::Duration::from_secs(5));
    server.set_write_timeout(std::time::Duration::from_secs(5));
    server.set_request_timeout(Some(std::time::Duration::from_secs(30)));
    server.set_log_file("./yar_server.log");
    server.set_pid_file("./yar_server.pid");

    // --- 注册 handler ---
    server
        .register_handler("echo", |req| {
            let mut pk = Packager::new();
            pk.push_string(b"echo:ok").unwrap();
            YarResponse::success(req.id(), pk)
        })
        .await;

    // --- 启动（不返回，直到出错） ---
    let _ = server.run().await;
}
```

## 监听地址

| 格式 | 说明 |
|---|---|
| `host:port` | TCP 监听，例如 `127.0.0.1:9501`、`0.0.0.0:9501`。**必须包含端口号** |
| `/path/to.sock` | Unix domain socket（仅 Unix 平台；Windows 上返回错误）。socket 文件由 `run()` 创建 |
| `http://…` / `https://…` | 直接报错。Yar 使用原始 TCP 协议，不支持 HTTP 传输 |

## 注册 handler：`register_handler`

```rust
server
    .register_handler("方法名", |req: &YarRequest| -> YarResponse {
        // 1) 从 req.get_parameters() 读取参数
        // 2) 业务逻辑
        // 3) 用 Packager 构造返回值，返回 YarResponse
        todo!()
    })
    .await; // 注意：注册是 async 的（内部是并发安全的 HashMap）
```

规则与约束：

1. **闭包签名**：`Fn(&YarRequest) -> YarResponse`，同步闭包（不能 `.await`）。
   耗时的 CPU 任务会阻塞该连接的后续请求，建议放到线程池（`tokio::task::spawn_blocking`）然后再返回。
2. **同名覆盖**：重复注册同名方法会静默替换旧实现（与 C 版行为一致）。
3. **handler 在哪个线程执行**：服务端为每条 TCP 连接启动一个 Tokio task，
   在该 task 内同步执行 handler；多个连接可并行服务。

## 在 handler 中读取参数

客户端发送的参数总是被包进一个 msgpack **数组**：`p = [arg0, arg1, ...]`。
服务端通过 `req.get_parameters()` 拿到这个数组（详细类型操作见[数据模型](data.md)）。

```rust
server
    .register_handler("add", |req| {
        // p 为数组：[a, b]
        let arr = req
            .get_parameters()
            .and_then(|p| p.as_array())
            .unwrap_or(&[]); // 安全兜底

        let a = arr.first().and_then(|v| v.as_integer()).unwrap_or(0);
        let b = arr.get(1).and_then(|v| v.as_integer()).unwrap_or(0);

        let mut pk = Packager::new();
        pk.push_long(a + b).unwrap();
        YarResponse::success(req.id(), pk)
    })
    .await;
```

**无参数调用**：客户端没有传参数时，`p` 是 Null（不是空数组！
`get_parameters()` 仍会返回 `Some(YarData::Null)`）：
`as_array()` 返回 `None`，所以上面 `.unwrap_or(&[])` 的写法天然兼容。

**map / 结构体参数**：例如客户端传 `{ "user": "alice", "age": 30 }`：

```rust
server
    .register_handler("register", |req| {
        let mut name = "anonymous";
        let mut age: Option<i64> = None;

        if let Some(arr) = req.get_parameters().and_then(|p| p.as_array()) {
            if let Some(first) = arr.first() {
                if let Some(map) = first.as_map() {
                    for (k, v) in map {
                        match k.as_str() {
                            Some("user") => name = v.as_str().unwrap_or("anonymous"),
                            Some("age") => age = v.as_integer(),
                            _ => {}
                        }
                    }
                }
            }
        }

        let mut pk = Packager::start_map(3).unwrap();
        pk.push_string(b"user").unwrap();
        pk.push_string(format!("{}@yar", name).as_bytes()).unwrap();
        pk.push_string(b"adult").unwrap();
        pk.push_bool(age.map(|a| a >= 18).unwrap_or(false)).unwrap();
        pk.push_string(b"alias").unwrap();
        pk.push_array(2).unwrap(); // 嵌套结构见《数据模型》
        pk.push_string(b"al1").unwrap();
        pk.push_string(b"al2").unwrap();

        YarResponse::success(req.id(), pk)
    })
    .await;
```

## 返回错误

返回值 `s` 字段（status）非 0 即表示失败，同时把错误信息写入 `e` 字段。
**status == 0 且 r 为返回值**才是成功响应。

```rust
server
    .register_handler("divide", |req| {
        let arr = req.get_parameters().and_then(|p| p.as_array()).unwrap_or(&[]);
        let b = arr.get(1).and_then(|v| v.as_integer()).unwrap_or(0);

        if b == 0 {
            let mut resp = YarResponse::new(req.id());
            resp.set_error(10000, "division by zero");
            return resp;
        }

        let a = arr.first().and_then(|v| v.as_integer()).unwrap_or(0);
        let mut pk = Packager::new();
        pk.push_long(a / b).unwrap();
        YarResponse::success(req.id(), pk)
    })
    .await;
```

- 错误信息超长（> 1024 字节）会被截断并追加 `"..."`（与 C 一致）。
- 未注册的方法被调用时，服务端会自动返回 status != 0、内容为
  `"Method not found: xxx"` 的错误响应。

## 运行选项

| 方法 | 默认值 | 说明 |
|---|---|---|
| `set_max_request_size(bytes)` | 10 MiB | 请求体上限（与 C 的 `YAR_MAX_BODY_SIZE` 一致）。超限请求在读取 body 前被拒绝 |
| `set_max_inflight(n)` | 1024 | 同时服务的连接数上限；`0` 表示不限制。超出后 accept 循环等待 |
| `set_read_timeout(Duration)` | 3 秒 | 单次 socket 读取的 idle 超时。超时后**关闭该连接** |
| `set_write_timeout(Duration)` | 5 秒 | 写响应的超时；客户端停止读取时不会永久占用任务 |
| `set_request_timeout(Option<Duration>)` | 30 秒 | 接收一个完整请求（头 + body）的总时限；`None` 关闭 |
| `set_log_file(path)` | None | 日志文件路径；未设置时输出 stderr |
| `set_pid_file(path)` | None | 启动时把当前进程 PID 写入该文件 |

## 连接生命周期与并发模型

- **每条 TCP 连接一个 Tokio task**；连接内串行处理请求（读请求 → handler → 写响应）。
- **非持久连接**：响应完一个请求后**立即关闭**连接。
- **持久连接**：客户端请求头带 `PROTOCOL_PERSISTENT` 标志时，连接进入循环，
  持续处理新请求，直到任意读超时/对端断开，或收到非持久请求（响应后关闭）。
- 客户端在不同连接之间天然并行；同一连接上的调用串行。

> 服务端响应头的 provider/token 字段为全零，`reserved` 为 0（非持久响应）。

## 关闭服务

当前版本没有内置的 graceful-shutdown API。常规做法：

1. 应用层自行捕获信号（如 `SIGINT`）；
2. 用 `tokio::select!` 与 `server.run()` 并行等待关闭信号；
3. `run()` 虽然"永不返回"，但把任务 abort 或整个进程退出即可（连接断开后客户端会收到 `Io`/`Connection` 错误）。
