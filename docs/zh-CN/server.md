# 服务端

[返回文档索引](README.md)

`YarServer` 是异步 Tokio 服务：持有一个 listener，接受连接，并为每条连接启动一个
task。连接 task 串行读请求、调用 handler、写响应；客户端带持久标志时继续循环。
没有 pre-fork、没有 daemon、也没有需要调的 worker 进程数——并行度由 Tokio 的
worker 线程决定。

```rust,no_run
use std::time::Duration;
use yar_rs::logger::LogLevel;
use yar_rs::{Packager, YarResponse, YarServer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = YarServer::new("0.0.0.0:9501");
    server.set_read_timeout(Duration::from_secs(3));
    server.set_max_inflight(1024);
    server.set_log_file("/var/log/yar-rs.log");
    server.set_log_level(LogLevel::Notice);
    server.set_access_log(true);
    server.set_pid_file("/run/yar-rs.pid");

    server
        .register_handler("add", |req| {
            let arr = req.get_parameters().and_then(|p| p.as_array()).unwrap_or(&[]);
            let sum: i64 = arr.iter().filter_map(|v| v.as_integer()).sum();
            let mut pk = Packager::new();
            pk.push_long(sum).unwrap();
            YarResponse::success(req.id(), pk)
        })
        .await;

    server.run().await?;
    Ok(())
}
```

## Handler

handler 是同步闭包 `Fn(&YarRequest) -> YarResponse`，按方法名注册；同名重复注册会
静默替换。发送前服务端会把响应 id 覆盖为请求 id，handler 可以留 0。

handler 在当前连接 task 上执行，也就是占用一个 Tokio worker 线程。阻塞或 CPU
密集的工作应移出该线程（多线程 runtime 下可用
`tokio::task::block_in_place`，或自己的线程池），避免拖住其他连接。handler 不能
`.await`，签名刻意保持同步，与 C 回调模型一致。

业务失败用响应表达：

```rust,ignore
let mut resp = YarResponse::error(req.id(), 1000, "division by zero");
```

成功用 status 0。服务端自动生成的错误（body 超限、请求解析失败、未知方法、
不支持的 packager）使用 status **8**（与 C 一致），错误文本截断为 1023 字节。

## 配置

| 方法 | 默认值 | 作用 |
|---|---|---|
| `set_max_request_size(bytes)` | 10 MiB | 声明长度超限时在读取 body 前拒绝 |
| `set_max_inflight(n)` | 1024 | 连接级闸门；permit 持有到连接结束；`0` 关闭限制 |
| `set_read_timeout(Duration)` | 3 秒 | 每次 socket 读的 idle 超时；持续慢速发送不会触发 |
| `set_request_timeout(Option<Duration>)` | 30 秒 | 一次请求（头 + body）的总时限；`None` 关闭 |
| `set_write_timeout(Duration)` | 5 秒 | 客户端停止读取时不会永久占住 task |
| `set_log_file(path)` | stderr | 日志目标（stderr、文件路径或 `|command` 管道） |
| `set_log_level(LogLevel)` | `Notice` | 最低级别；OKEY 始终输出 |
| `set_access_log(bool)` | `true` | 每请求 OKEY/ERROR 行 |
| `set_pid_file(path)` | 无 | listener 绑定后写入进程 PID |

监听地址来自 `YarServer::new`：

- `host:port` — TCP。
- `tcp://host:port` — 同上，接受 C 风格前缀。
- `/path/to.sock` — Unix domain socket（仅 Unix；其他平台返回不支持错误）。

## 连接生命周期

1. `read_timeout` 约束每次读；`request_timeout` 约束整个头+body 交换。每个请求重新计时。
2. 声明长度通过 `max_request_size` 后才读取 body。
3. packager 标签必须是 `MSGPACK`；否则回 status 8 错误响应，非持久连接随后关闭。
4. 解析请求 envelope；失败则回错误响应。
5. 按方法名查 handler；未知方法回 status 8，`"Method not found: ..."`。
6. 在 `write_timeout` 内写响应；响应 id 回显请求 id，provider 为服务端名。
7. 非持久请求关闭连接；持久请求回到第 1 步。

## 访问日志

`set_access_log(true)`（默认）时，每个完成的请求写一行，字段与 C 一致：

```text
<id> <peer> "<method>" "<provider>" <bytes_sent> <elapsed_us>     # 成功
<id> <peer> "<method>" <error> "<provider>" <bytes_sent> -        # status != 0
```

方法名、错误、provider 中的 CR/LF 会被转义，防止对端伪造日志行。压测或高吞吐
场景可用 `set_access_log(false)` 或二进制的 `--no-access-log` 关闭。

## 关闭

`run()` 一直服务到进程退出或初始化失败。没有 graceful drain API：停止进程会关闭
listener 并丢弃连接 task。请用进程管理器停止服务；这是有意的取舍。

## 未实现

- daemonize、pre-fork worker、worker 重启、uid/gid 降权。
- 可选 JSON packager（收到 JSON tag 时返回与 C 未编译 cJSON 时相同的错误响应）。
- `PROTOCOL_PING` / `PROTOCOL_LIST`（仅常量，与 C 服务端一致）。

设计取舍与完整的 C 兼容性对照见 [FAQ](faq.md)。
