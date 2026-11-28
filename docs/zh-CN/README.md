# yar-rs 中文文档

[English](../README.md) | **中文**

`yar-rs` 是 Yar RPC 协议的 Rust 实现。它是独立项目，而不是 C 代码或 C 文档的移植：
服务端是基于 Tokio 的异步服务，客户端是阻塞 API，数据模型是安全的 Rust 树。
线上格式与原始 C 实现保持兼容，因此 `yar-rs` 服务端可以服务 C / PHP / Java / Rust
客户端，`yar-rs` 客户端也可以调用它们。

协议的权威参考仍是原始 C 项目：
[laruence/yar-c](https://github.com/laruence/yar-c) 与 PHP 扩展
[laruence/yar](https://github.com/laruence/yar)。本中文文档按 Rust 设计编写，
不是 C 手册的翻译。

## 文档目录

| 章节 | 内容 |
|---|---|
| [快速上手](getting-started.md) | 安装 crate、最小服务端/客户端、运行仓库示例 |
| [服务端](server.md) | `YarServer` 设计、handler、连接生命周期、超时、限制、Unix socket、访问日志 |
| [客户端](client.md) | `YarClient` 地址、超时、持久连接、失败处理、预打包请求 |
| [数据模型](data.md) | `Packager`、`Unpackager`、`YarData`、msgpack 类型、解析限制 |
| [协议](protocol.md) | 线上头、请求/响应 envelope、状态码、与 C/PHP/Java 互通 |
| [错误与日志](errors-logging.md) | `YarError`、业务错误与传输错误、日志级别、访问日志 |
| [FAQ](faq.md) | 设计取舍、限制、常见问题 |
| [基准测试](https://github.com/lovelock/yar-rs/blob/master/profile/README.md) | C vs Rust 压测方法、资源对齐、报告列与结果 |

## 设计概览

- **服务端**：一个 Tokio listener 接受连接，每条连接一个 task；连接内串行读取请求、
  调用已注册的同步 handler、写回响应，持久连接则继续循环。没有 pre-fork / daemon。
  连接闸门与 idle/总 deadline 限制资源占用。
- **客户端**：阻塞 API，一条连接同时只有一次在途调用，与 C 客户端模型一致；
  支持持久连接。调用失败后连接会被标记为不可复用，避免读到迟到的响应。
- **数据**：msgpack 字节解析为 `YarData`，由 `Packager` 写回。解析器限制嵌套深度、
  节点数、声明长度，并拒绝 trailing bytes。
- **默认值**（均可配置）：请求/响应 10 MiB、1024 在途连接、3 秒 idle 读超时、
  30 秒请求总 deadline、5 秒写超时、客户端 1 秒超时、服务端错误 status 8、
  每请求 OKEY 访问日志。

## 阅读顺序

1. 第一次使用：先读[快速上手](getting-started.md)，再读[客户端](client.md)的持久连接一节。
2. 编写 handler：[服务端](server.md)与[数据模型](data.md)。
3. 对接已有 Yar 服务：[协议](protocol.md)。
4. 排查问题：[错误与日志](errors-logging.md)与 [FAQ](faq.md)。

## 与 C 参考实现的关系

| 方面 | yar-rs | C 参考实现 |
|---|---|---|
| 服务端并发 | Tokio task / 每连接一个，单进程 | pre-fork 事件循环 worker |
| 客户端 | 阻塞，每连接一次在途调用 | 阻塞，同一模型 |
| 线上格式 | 相同的 82 字节头 + `MSGPACK\0` + msgpack | 参考实现 |
| 服务端错误 status | 8（`YAR_ERROR`） | 8 |
| 错误信息上限 | 1023 字节 | 1023 字节 |
| 请求体上限 | 10 MiB | 10 MiB |
| 读超时 | 3 秒 idle + 30 秒总时限（可配） | 3 秒/事件（可配） |
| 访问日志 | 每请求 OKEY/ERROR，默认开 | 每请求 OKEY/ERROR |
| 可选 JSON packager | 未实现 | 构建时可选 |
| daemon/pre-fork/uid-gid | 未实现（有意取舍） | 支持 |

兼容细节见[协议](protocol.md)与 [FAQ](faq.md)；线上格式以 C 源码与文档为准。

## 仓库结构

```
src/        协议、pack、request、response、client、server、logger
examples/   cross_server / cross_client
tests/      单元、集成、安全、文档示例测试
profile/    C vs Rust 基准工具与报告（不随 crate 发布）
docs/       英文文档（根目录）与中文文档（docs/zh-CN）
```
