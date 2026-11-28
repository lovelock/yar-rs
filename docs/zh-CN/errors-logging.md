# 错误与日志

[返回文档索引](README.md)

## 错误

客户端与底层 API 返回 `YarResult<T> = Result<T, YarError>`。

| 变体 | 场景 |
|---|---|
| `Protocol(String)` | 字节不符合协议（截断、不是 map、缺字段、过大） |
| `Pack(String)` | msgpack 编解码失败，包括解析限制 |
| `Io(std::io::Error)` | socket 读/写失败 |
| `Connection(String)` | 连接失败、对端关闭、或复用了已 poison 的 client |
| `Timeout` | socket 或服务端 deadline 到期 |
| `InvalidMagic` | 头部 magic 不匹配 |
| `UnsupportedPackager(String)` | 对端使用了 `MSGPACK` 之外的 packager |
| `MethodNotFound(String)` | 服务端没有该方法的 handler（以 status 8 响应返回） |

**业务错误不是 `Err`**。服务端失败是一次成功调用，只是
`YarResponse::get_status()` 非 0：

```rust,ignore
let response = client.call("divide", vec![a, b])?; // 只代表传输/解码成功
if response.get_status() == 0 {
    let value = response.get_response();
} else {
    eprintln!("status {}: {:?}", response.get_status(), response.get_error());
}
```

服务端自动生成的错误使用 status 8，错误文本截断为 1023 字节。handler 可用任意
status；`YarResponse::error(id, code, message)` 同样截断消息。

传输、超时或解码错误后，客户端会 poison 连接：socket 里可能还有迟到的响应。
下一次调用前请重新 `YarClient::connect`。

## 日志

`YarLogger` 的行格式：

```text
{hostname} [{YYYY-MM-DD HH:MM:SS}] {LEVEL}: {message}
```

级别（数值与 C logger 对齐）：

| 级别 | 数值 | 说明 |
|---|---:|---|
| `LogLevel::Ok` | 0 | 访问日志，输出名 `OKEY`；不受阈值过滤 |
| `LogLevel::Debug` | 1 | 协议/流程细节 |
| `LogLevel::Notice` | 2 | 服务端默认阈值 |
| `LogLevel::Warning` | 4 | 警告 |
| `LogLevel::Error` | 8 | 错误 |

低于阈值的消息被抑制；启用访问日志时 `OKEY` 始终输出。

服务端在 `run()` 时根据 `set_log_file` 与 `set_log_level` 初始化进程级 logger：

```rust,ignore
server.set_log_file("/var/log/yar-rs.log"); // 或 "|command"；默认 stderr
server.set_log_level(LogLevel::Notice);
server.set_access_log(true);
```

也可以自行调用 `logger::init_global(path, level)`；每个进程只能成功一次。
`YarLogger::init` 可创建不挂全局的独立实例。

### 访问日志

`set_access_log(true)`（默认）时，每完成一个请求写一行，字段与 C 一致：

```text
<id> <peer> "<method>" "<provider>" <bytes_sent> <elapsed_us>     # 成功
<id> <peer> "<method>" <error> "<provider>" <bytes_sent> -        # status != 0
```

方法名、错误、provider 中的 CR/LF 会被转义。压测或高吞吐部署可关闭：

```rust,ignore
server.set_access_log(false);
// 或使用自带二进制：
// yar-server-rust --no-access-log
```

`yar-server-rust` 也支持 `--log-file` 与 `--log-level`。
