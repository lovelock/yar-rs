# FAQ 与迁移指南

## 从 C 版 yar 迁移：API 对照表

| C API | yar-rs 对应 |
|---|---|
| `yar_server_init("127.0.0.1:8888")` | `YarServer::new("127.0.0.1:8888")` |
| `yar_server_register_handler(handlers[])` | `server.register_handler("name", 闭包).await`（同步闭包 `Fn(&YarRequest) -> YarResponse`） |
| `yar_server_run()` | `server.run().await` |
| `yar_server_set_opt(YAR_READ_TIMEOUT, &t)` | `server.set_read_timeout(Duration)` |
| `YAR_LOG_FILE` | `server.set_log_file(path)` |
| `YAR_MAX_CHILDREN` (pre-fork worker) | `server.set_max_workers(n)`（目前为占位，未参与调度） |
| `YAR_PPID/CHILD_INIT` hooks | 暂未实现（Rust 版无 pre-fork） |
| `YAR_PID_FILE` | `server.set_pid_file(path)`（当前仅记录，不创建） |
| `yar_client_init("tcp://host:port")` | `YarClient::connect("host:port")` |
| `client->call(client, method, n, args)` | `client.call(method, Vec<Packager>)` |
| `yar_client_set_opt(YAR_TIMEOUT, &t)` | `client.set_timeout(Duration)` |
| `YAR_PERSISTENT_LINK` | `client.set_persistent(bool)` |
| `yar_client_destroy(client)` | `client.close()`（消费型） |
| `yar_pack_start_map(n)/start_array(n)` | `Packager::start_map(n) / start_array(n)` |
| `yar_pack_push_*` | `pk.push_bool / push_long / push_ulong / push_double / push_string / push_array / push_map / push_null / push_data / push_packager` |
| `yar_pack_to_string(pk, &payload)` | `pk.into_bytes()` / `pk.as_bytes()` |
| `yar_unpack_data_*`, `yar_unpack_iterator_*` | `YarData.as_*()` / `YarData.iter()`（详见[数据模型](data.md)） |
| `yar_request_pack/unpack/set_parameters` | `YarRequest::pack/unpack/set_parameters` |
| `yar_response_set_error/set_retval/get_status` | `YarResponse::set_error / set_retval / get_status` |

## 常见问题

### Q1：`call` 报 `Connection("connection closed: expected 82 bytes, got 0")`

非持久模式下服务端响应一次后就关闭连接（与 C 服务端行为一致），第二次 `call`
复用了已关闭的连接。解决：`client.set_persistent(true)`，或每次调用新建 client。

### Q2：`connect("localhost")` 失败：must include port

TCP 地址必须写成 `host:port`。

### Q3：`connect("http://...")` 失败：HTTP/HTTPS not supported

Yar 是原始 TCP 协议。Unix socket 客户端可用 `/path/to.sock`；
（服务端的 Unix socket 支持尚未实现。）

### Q4：handler 里读到的整数跟推送的类型不一致

msgpack 紧凑编码导致"小正数解包为 Ulong"——
用 `as_integer()` 统一读取（详见[数据模型](data.md)的"整数解码陷阱"一节）。

### Q5：如何在 `#[tokio::main]` 里发起调用

`call()` 是阻塞的，放进 `tokio::task::spawn_blocking`（见[客户端指南](client.md#在-async-代码中使用)）。

### Q6：handler 能不能是 async fn

不能。当前 handler 是同步闭包；耗时 CPU/IO 工作请用 `spawn_blocking` 或
线程池处理后返回结果。

### Q7：服务端能同时服务多少连接

每连接一个 Tokio task，理论无上限（受内存/句柄限制）。`set_max_workers` 目前
不参与调度（占位 API）。

### Q8：多请求并发/请求 ID

Rust 客户端沿用了 C 客户端的实现，请求 id 固定为 `1000`，同一连接同一时刻
只有一次在途调用；id 匹配、多路复用是不支持的。

### Q9：服务端收到了什么请求会断开连接

- 读取头部/体失败（对端断开、超时）；
- 请求头魔数不合法；
- body 长度小于 8 字节无法容纳 packager 名；
- packager 名不是 `b"MSGPACK\0"`；
- 请求体解码失败（此时先回错误响应再断开）；
- 非持久请求完成响应后（正常路径）。

### Q10：header 字段怎么读

`YarHeader` 是 packed 结构，数值字段请通过 `to_bytes()`/`from_bytes()`
在大端线上字节上读取；直接 `header.id` 之类的字段访问无效（对齐问题）。
说明见[协议页](protocol.md)。

## 已知限制（如实列出）

1. 服务端不支持 Unix domain socket（客户端支持）；
2. 服务端没有 graceful shutdown API、没有 pre-fork/daemon 模式（`set_stand_alone(false)` 无差别）；
3. `PROTOCOL_PING` / `PROTOCOL_LIST` 仅保留常量，未实现；
4. 服务端读 `body_len` 全部缓冲后才解析，恶意大包可能占用内存 —— 请部署在可信网络或前置限流；
5. 错误信息 1024 字节截断、provider/token 只有前 16 字节有效 —— 与 C 版一致；
6. 客户端为同步阻塞 API，async 环境需转发到阻塞线程。

## 文档示例的正确性保障

[tests/doc_examples.rs](../tests/doc_examples.rs) 将本目录每个文档片段
实现为可执行测试，`cargo test` 会：

1. 编译检查示例代码的**每个 API 调用签名**；
2. 真实启动服务端并验证行为断言；

贡献者修改 API 或文档时必须同步更新该文件，保证 doc 与代码不漂移。
