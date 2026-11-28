# yar-rs 用户指南

本目录是 [yar-rs](https://github.com/lovelock/yar-rs) —— [Yar RPC 框架](https://github.com/laruence/yar) 的 Rust 实现 —— 的完整用户指南。

> **可编译的示例都在测试中校验**：标记为 `rust` 且可独立运行的片段对应
> [`tests/doc_examples.rs`](../tests/doc_examples.rs) 中的测试；纯片段用
> `text` 标记，说明性代码不保证可独立编译。如果你要修改文档中的示例，请同步
> 修改对应测试。

## 指南目录

| 章节 | 内容 |
|---|---|
| [快速上手](getting-started.md) | 安装、最小可运行的 client/server 示例、运行仓库自带 demo |
| [服务端](server.md) | `YarServer` 全部选项、handler 编写规范、持有连接、并发模型 |
| [客户端](client.md) | `YarClient` 连接格式、超时、持久连接、参数打包与错误处理 |
| [数据模型](data.md) | `Packager` / `Unpackager` / `YarData`，嵌套打包与迭代器 |
| [协议与跨语言互通](protocol.md) | 线上字节格式、msgpack 帧结构、与 PHP / Java / C 客户端对接 |
| [错误与日志](errors-logging.md) | `YarError` 枚举、`YarLogger` 日志系统、访问日志格式 |
| [FAQ 与迁移](faq.md) | 常见问题、C API → Rust API 对照表、C 兼容性差异、已知限制 |
| [基准测试](../profile/README.md) | C vs Rust 的 k6 风格矩阵、资源对齐规则、报告列与实测结果 |

## 当前实现要点

- **服务端**：错误统一 status 8（C 的 `YAR_ERROR`）；每请求 OKEY/ERROR
  访问日志（默认开启，`set_access_log(false)` 可关）；错误信息 1023 字节
  截断；接受 `tcp://` 与 Unix socket；10 MiB 请求上限；idle 读超时、
  请求总 deadline、响应写超时；默认 1024 在途连接上限。
- **客户端**：1 秒默认超时（与 C 一致）；10 MiB 响应上限；packager 与
  响应 id 校验；传输/解码失败后连接被标记为不可复用；`tcp://`；
  `build_request_packet` + `call_raw` 支持预打包请求。
- **解析/打包**：msgpack 深度、长度、节点数限制；`unpack` 直接移动数据；
  request/response `pack` 只做一次整包拷贝。
- **未实现**（刻意保留，见 [FAQ](faq.md)）：可选 JSON packager、
  daemon/pre-fork/uid-gid。

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
