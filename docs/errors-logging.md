# 错误与日志

> 本文示例在 `tests/doc_examples.rs` 中有编译校验：
> `doc_yar_error_matching`、`doc_handler_error_response`、`doc_logger_usage`。

## 一、错误枚举 `YarError`

所有客户端/底层 API 的返回值类型都是 `YarResult<T> = Result<T, YarError>`。

| 错误变体 | 打印前缀 | 出现在什么场景 |
|---|---|---|
| `Protocol(String)` | `Protocol error:` | 收到的数据不符合协议（截断、不是 map、缺字段等），如"unexpected EOF while reading 82 bytes" |
| `Pack(String)` | `Pack error:` | msgpack 打包/解析失败 |
| `Io(std::io::Error)` | `IO error:` | 网络读写 IO 失败 |
| `Connection(String)` | `Connection error:` | 连接失败；`call` 期间对端关闭（如"connection closed: expected 82 bytes, got 0" —— 非持久 client 调第二次 call 的典型症状）；`http://` 地址等 |
| `Timeout` | `Timeout` | （保留变体） |
| `InvalidMagic` | `Invalid magic number` | 对端发来的不是 Yar 协议（魔数校验失败） |
| `UnsupportedPackager(String)` | `Unsupported packager:` | 对端使用了非 `MSGPACK\0` 的打包器名 |
| `MethodNotFound(String)` | `Method not found:` | **服务端**未注册该方法（自动回包给客户端，客户端在 `status`/`error` 里看到此消息） |

匹配示例：

```rust
use yar_rs::{YarClient, YarError};

let err = YarClient::connect("http://example.com:80").unwrap_err();
match err {
    YarError::Connection(msg) => assert!(msg.contains("HTTP")),
    other => panic!("expected Connection error, got {:?}", other),
}

// 无端口号的 host 会被拒绝
let err2 = YarClient::connect("localhost").unwrap_err();
assert!(matches!(err2, YarError::Connection(_)));
```

## 二、业务错误与传输错误的分界

**业务错误不产生 `Err`，只体现在响应状态里**：

```text
// 服务端 handler：
let mut resp = YarResponse::new(req.id());
resp.set_error(10000, "division by zero");
return resp;                     // status != 0，客户端收到的 ok(resp) 里有错误信息

// 客户端：
let resp = client.call("divide", vec![a, b])?;   // 传输 OK 返回 Ok
assert_eq!(resp.get_status(), 10000);            // 用 status 判断
assert_eq!(resp.get_error(), Some("division by zero"));
assert!(resp.get_response().is_none());          // 出错时没有返回值
```

错误信息超过 1024 字节会被截断并追加 `"..."`（与 C 结果一致）。

## 三、日志系统 `logger` 模块

Yar 的日志系统是自带的独立实现（与 log/env_logger 等生态无关），格式
（与 C 版本共享）：

```text
{hostname} [{YYYY-MM-DD HH:MM:SS}] {LEVEL}: {message}
```

### 级别与过滤规则

`logger::LogLevel`（数值与 C 版对齐）：

| 级别 | 数值 | 说明 |
|---|---:|---|
| `LogLevel::Ok` | 0 | 永远输出（不受阈值过滤） |
| `LogLevel::Debug` | 1 | 调试信息 |
| `LogLevel::Notice` | 2 | 常规通知（服务端默认阈值） |
| `LogLevel::Warning` | 4 | 警告 |
| `LogLevel::Error` | 8 | 错误 |

过滤规则：`level < 阈值` 的消息被抑制；`Ok` 永远输出。

### 全局 logger

服务端 `run()` 时若全局 logger 未初始化会**自动**用配置（log_file + `Notice`）
初始化全局 logger；也可以在自己的代码中提前初始化：

```rust
use yar_rs::logger::{init_global, set_hostname, log_global, LogLevel};

// None -> stderr；其他支持文件路径追加，或 "|cmd" 管道命令
init_global(None, LogLevel::Notice).unwrap_or(()); // 已初始化时返回 Err，首次调用成功
set_hostname("my-yar-service");
log_global(LogLevel::Notice, "service started");

// 本地使用也可以独立实例化（不挂全局）
let local = yar_rs::logger::YarLogger::init(None, LogLevel::Debug).unwrap();
local.log(LogLevel::Error, "local only");

assert!(local.is_enabled());
assert_eq!(local.min_level(), LogLevel::Debug);
```

注意：

- `init_global` 全进程只能成功调用一次（内部用 `OnceLock`），重复调用返回
  `Err("global logger already set")`；上面用 `.unwrap_or(())` 忽略。
- 服务端启动时 `set_log_file` 路径可能无法创建 → `run()` 返回
  `Protocol("Failed to initialize logger: ...")`。
