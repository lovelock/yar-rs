# yar-rs 用户指南

本目录是 [yar-rs](https://github.com/laruence/yar-rs) —— [Yar RPC 框架](https://github.com/laruence/yar) 的 Rust 实现 —— 的完整用户指南。

> **所有 doc 示例均经过编译与运行校验**：每个文档片段都对应
> [`tests/doc_examples.rs`](../tests/doc_examples.rs) 中的一个测试，
> 由 `cargo test` 保持同步。如果你要修改本文档中的示例，请同步修改对应测试。

## 指南目录

| 章节 | 内容 |
|---|---|
| [快速上手](getting-started.md) | 安装、最小可运行的 client/server 示例、运行仓库自带 demo |
| [服务端](server.md) | `YarServer` 全部选项、handler 编写规范、持有连接、并发模型 |
| [客户端](client.md) | `YarClient` 连接格式、超时、持久连接、参数打包与错误处理 |
| [数据模型](data.md) | `Packager` / `Unpackager` / `YarData`，嵌套打包与迭代器 |
| [协议与跨语言互通](protocol.md) | 线上字节格式、msgpack 帧结构、与 PHP / Java / C 客户端对接 |
| [错误与日志](errors-logging.md) | `YarError` 枚举、`YarLogger` 日志系统 |
| [FAQ 与迁移](faq.md) | 常见问题、C API → Rust API 对照表、已知限制 |

## 推荐阅读顺序

1. **第一次使用**：先完整阅读 [快速上手](getting-started.md) 和 [客户端](client.md)（特别是"持久连接"一节）。
2. **编写 handler**：读 [服务端](server.md) 与 [数据模型](data.md)——数据模型一节解释了所有类型读取的细节（例如整数的 `Ulong/Long` 陷阱）。
3. **对接已有 Yar 服务**（PHP / Java 生态）：读 [协议与跨语言互通](protocol.md)。
4. **排查问题**：看 [FAQ](faq.md)。

## 一分钟了解 Yar 协议

一次 RPC 调用在线上的样子：

```text
客户端发出:
┌─────────────────── 82 字节协议头 (大端) ───────────────────┬───────── 8B ──────────┬──── msgpack 请求体 ────┐
│ id │ version │ magic=0x80DFEC60 │ flags │ provider │ token │ body_len │  "MSGPACK\0"  │ {"i":id,"m":"方法","p":参数} │
└───────────────────────────────────────────────────────────────────────────────┴────────────────┴────────────────────────┘

服务端返回:
┌─────────────────── 82 字节协议头 ───────────────────┬──── 8B ────┬─────────────── msgpack 响应体 ───────────────┐
│                  ... 同上 ...                       │ "MSGPACK\0" │ {"i":id,"s":状态,"r":返回值,"e":错误信息}    │
└─────────────────────────────────────────────────────┴─────────────┴──────────────────────────────────────────────┘
```

保持连接（persistent）时客户端可以在同一条 TCP 连接上连续完成多次调用。
详见[协议与跨语言互通](protocol.md)。
