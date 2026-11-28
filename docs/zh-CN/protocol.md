# 协议与跨语言互通

[返回文档索引](README.md)

`yar-rs` 使用经典 Yar 线上协议：82 字节头、8 字节 packager 名、msgpack body。
协议由原始 C 项目定义（[laruence/yar-c](https://github.com/laruence/yar-c)）；
本页说明 Rust 侧表示与互通要点。

## 帧

```text
+--------------------------- 82 字节头 --------------------------------+
| id u32 | version u16 | magic u32 | flags u32 | provider [u8;32]      |
| token [u8;32]                                                       |
| body_len u32                                                       |
+--------------------------------------------------------------------+
| packager 名：8 字节，"MSGPACK\0"                                    |
+--------------------------------------------------------------------+
| msgpack body（请求或响应 envelope）                                 |
+--------------------------------------------------------------------+
```

`body_len` 统计 packager 名与 msgpack body。头中的数值字段在线上为大端；
`version` 为 0，magic 为 `0x80DFEC60`。

`YarHeader` 内部以**主机字节序**保存数值字段，用 `to_bytes()` /
`from_bytes()` 转换。provider 与 token 最多写 31 字节并保留 NUL 结尾，与 C 的
`strncpy` 行为一致。

## 标志位

| 位 | 常量 | 含义 |
|---|---|---|
| `0x1` | `PROTOCOL_PERSISTENT` | 客户端希望响应后保持连接 |
| `0x2` | `PROTOCOL_PING` | 保留；两端服务端都未实现 |
| `0x4` | `PROTOCOL_LIST` | 保留；两端服务端都未实现 |

非持久请求响应后服务端关闭连接；持久请求继续读取下一帧。

## 请求 envelope

一个含 3 个 key 的 msgpack map：

```text
{
  "i": <ulong>,        请求 id
  "m": <string>,       方法名
  "p": <any or null>   参数
}
```

`YarClient::call` 会把参数包成一个数组，因此 handler 通常看到
`p = [arg0, arg1, ...]`。服务端在响应中回显请求 id。

## 响应 envelope

```text
{
  "i": <ulong>,         请求 id（回显）
  "s": <int>,           状态码：0 = 成功，非 0 = 错误
  "r": <any or null>,   返回值
  "e": <string or null> 错误信息
}
```

服务端自动生成的错误使用 status **8**（C 的 `YAR_ERROR`）：body 超限、请求格式
错误、未知方法、不支持的 packager。错误文本截断为 1023 字节（1020 + `...`），
与 C 的 1024 字节缓冲区一致。handler 可以用任意 status 表示业务错误。

## 尺寸与限制

| 项目 | 值 |
|---|---|
| 头 | 82 字节 |
| packager 名 | 8 字节，`MSGPACK\0` |
| 请求体上限 | 默认 10 MiB（`set_max_request_size`） |
| 响应体上限 | 客户端默认 10 MiB（`set_max_response_size`） |
| 解析深度 / 节点数 | 128 / 262,144 |
| 在途连接 | 默认 1024（`set_max_inflight`） |

请求上限在读取 body 前检查；客户端在分配前检查响应上限。它们与[数据模型](data.md)
中的解析器限制是两套独立限制。

## 互通

帧与 envelope 与 C 实现一致，因此以下双向可用：

- `yar-rs` 服务端 + C / PHP / Java / Rust 客户端；
- `yar-rs` 客户端 + C / PHP / Java / Rust 服务端。

packager 名必须是 `MSGPACK`。部分 C 构建的可选 JSON packager 未实现；带 JSON tag
的请求会收到与 C 未编译 cJSON 时相同的错误响应。详见 [FAQ](faq.md)。

### 与 C 参考实现的差异

| 项目 | yar-rs | C 参考实现 |
|---|---|---|
| 服务端并发 | 每连接一个 Tokio task，单进程 | pre-fork 事件循环 worker |
| 默认读超时 | 3 秒 idle + 30 秒总时限 | 3 秒/事件 |
| 写超时 | 5 秒 | 服务端 timeout |
| 服务端错误 status | 8 | 8 |
| 错误文本截断 | 1023 字节 | 1023 字节 |
| `tcp://` 地址 | 接受 | 接受 |
| Unix socket | 服务端与客户端（仅 Unix） | 支持 |
| 访问日志 | 每请求 OKEY/ERROR，默认开 | 每请求 OKEY/ERROR |
| 可选 JSON packager | 未实现 | 可选 |
| daemon/pre-fork/uid-gid | 未实现 | 支持 |
| PING/LIST 标志 | 仅常量 | 仅常量 |

线上格式与 PHP 客户端行为的权威描述仍以 C 源码与文档为准。

## provider 与 token

客户端在 `provider` 中发送自己的名字（`Yar(Rust)-0.1.0`）；服务端响应写自己的
名字。`token` 在 Rust 实现中不使用，发送全零。两者都不影响路由或鉴权。
