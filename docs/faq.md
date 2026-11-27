# FAQ 与迁移指南

## 从 C 版 yar 迁移：API 对照表

| C API | yar-rs 对应 |
|---|---|
| `yar_server_init("127.0.0.1:8888")` | `YarServer::new("127.0.0.1:8888")` |
| `yar_server_register_handler(handlers[])` | `server.register_handler("name", 闭包).await`（同步闭包 `Fn(&YarRequest) -> YarResponse`） |
| `yar_server_run()` | `server.run().await` |
| `yar_server_set_opt(YAR_READ_TIMEOUT, &t)` | `server.set_read_timeout(Duration)` |
| `YAR_LOG_FILE` | `server.set_log_file(path)` |
| `YAR_MAX_CHILDREN` (pre-fork worker) | `server.set_max_inflight(n)`（同时服务的连接数；不是进程数） |
| `YAR_PPID/CHILD_INIT` hooks | 暂未实现（Rust 版无 pre-fork） |
| `YAR_PID_FILE` | `server.set_pid_file(path)`（启动时写入当前 PID） |
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

Yar 是原始 TCP 协议。Unix socket 可用 `/path/to.sock`（客户端与服务端均支持，仅 Unix 平台）。

### Q4：handler 里读到的整数跟推送的类型不一致

msgpack 紧凑编码导致"小正数解包为 Ulong"——
用 `as_integer()` 统一读取（详见[数据模型](data.md)的"整数解码陷阱"一节）。

### Q5：如何在 `#[tokio::main]` 里发起调用

`call()` 是阻塞的，放进 `tokio::task::spawn_blocking`（见[客户端指南](client.md#在-async-代码中使用)）。

### Q6：handler 能不能是 async fn

不能。当前 handler 是同步闭包；耗时 CPU/IO 工作请用 `spawn_blocking` 或
线程池处理后返回结果。

### Q7：服务端能同时服务多少连接

每连接一个 Tokio task；默认 `set_max_inflight(1024)`，可设为 `0` 关闭限制。
同时受内存/文件句柄限制。

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

`YarHeader` 中的数值字段是**主机字节序**，可以直接读 `header.id` 等字段；
线上格式用 `to_bytes()` 生成、`from_bytes()` 解析（内部做大小端转换）。
说明见[协议页](protocol.md)。

## 已知限制（如实列出）

1. Windows 上不支持 Unix domain socket（TCP 可用）；
2. 服务端没有 graceful shutdown API、没有 pre-fork/daemon 模式；
3. `PROTOCOL_PING` / `PROTOCOL_LIST` 仅保留常量，未实现；
4. 服务端仍会把请求体完整缓冲后解析；默认 10 MiB 上限 + msgpack 深度/长度/节点限制，
   但高并发大包仍会占用相应内存，建议配合 `set_max_inflight` 或前置限流；
5. 错误信息超过 1024 字节会截断并追加 `"..."`；provider/token 最多 31 字节（与 C 一致）；
6. 客户端为同步阻塞 API，async 环境需转发到阻塞线程。

## 文档示例的正确性保障

[tests/doc_examples.rs](../tests/doc_examples.rs) 覆盖本目录中的主要可运行示例，
`cargo test` 会编译检查 API 签名，并对关键行为做断言。纯说明性片段用 `text`
标记。贡献者修改 API 或文档时应同步更新测试，避免文档漂移。
