# FAQ

[返回文档索引](README.md)

## 和原始 Yar 项目是什么关系？

`yar-rs` 用 Rust 实现 Yar RPC 协议，是独立项目，不是 C 代码或 C 文档的移植。
C 实现（[laruence/yar-c](https://github.com/laruence/yar-c)）与 PHP 扩展
（[laruence/yar](https://github.com/laruence/yar)）仍是线上格式与既有部署的
权威参考。帧和 envelope 一致，因此 `yar-rs` 服务端可以与 C/PHP/Java 客户端
（或反向）互通。

## 为什么服务端和 C 版看起来不一样？

C 服务端使用 pre-fork 的事件循环 worker；`yar-rs` 用单进程 + Tokio listener +
每连接一个 task。外部行为一致（并发持久连接、连接内串行请求），但并发模型是
Rust 原生的。没有 worker 数需要调，Tokio 按进程可用 CPU 决定线程数。

## 能 daemon 化或 pre-fork 吗？

不能。daemonize、pre-fork worker、worker 重启、uid/gid 降权均未实现。请用进程
管理器托管。PID 文件选项只为需要它的工具写当前 PID。

## 如何优雅关闭？

`YarServer::run()` 一直服务到进程退出。没有 drain API：停止进程会关闭 listener
并丢弃连接 task。用进程管理器的停止信号即可。

## 能服务多少连接？

默认 `max_inflight = 1024` 条连接。permit 持有到连接结束，因此持久连接会一直
占用名额。`set_max_inflight(0)` 关闭闸门。真正的限制是内存与文件描述符；每条
连接最多缓冲一个请求 body。

## handler 是同步的，耗时工作放哪？

handler 运行在连接 task 的 worker 线程上。阻塞或 CPU 密集工作应移出该线程
（多线程 runtime 下用 `tokio::task::block_in_place`，或自己的线程池）。handler
不能 `.await`；这样保持了与 C 回调模型一致的接口。

## 客户端为什么是阻塞的？

与 C 客户端一致，便于嵌入同步程序；async 中用 `tokio::task::spawn_blocking`。
每连接一次在途调用，没有请求 id 多路复用；并发时为每个任务建独立 client。

## 超时后为什么报 “poisoned”？

超时或传输错误可能在 socket 中留下迟到的响应。复用连接会把旧响应当成下一次
调用的结果，因此客户端在重新连接前拒绝继续调用。

## 为什么调用返回 `Ok` 但 status 非 0？

业务失败用响应表达，不是传输错误：检查 `YarResponse::get_status()`。服务端自动
生成的错误是 status 8。传输与解码失败才是 `Err`。

## 错误 status 和截断长度从哪来？

与 C 服务端一致：服务端自动错误 status 8（`YAR_ERROR`），错误文本截断为 1023
字节（1020 + `...`），以适配 C 的 1024 字节错误缓冲区。

## 支持 `tcp://`、Unix socket、Windows 吗？

客户端与服务端都接受 `tcp://host:port`。Unix domain socket 在 Unix 上双向支持，
其他平台返回不支持错误。TCP 在 Windows/macOS 可用；测试在三个平台都跑。

## 尺寸限制是多少？

请求体（服务端）与响应体（客户端）默认 10 MiB，均在分配前检查。msgpack 解析器
还限制嵌套 128 层、节点 262,144 个，拒绝超过剩余输入的长度与 trailing bytes。

## C 和 Rust 的值字节完全一样吗？

支持的类型一致：相同的紧凑整数编码、envelope、头部布局。见[协议](protocol.md)。
provider 名字与客户端实现名不同，但不影响路由。

## 基准里为什么没有 `4 MiB × 640 VUs`？

该单元仅服务端 body buffer 就需要 640 × 4 MiB > 4 GiB，2 GiB 服务端会被 OOM
kill。基准文档已说明，并用 `--sizes` / `--vus` 选择可行子集。见
[基准测试](https://github.com/lovelock/yar-rs/blob/master/profile/README.md)。

## 哪些功能没有实现？

- 可选 JSON packager（JSON tag 返回与 C 未编译 cJSON 时相同的错误）。
- `PROTOCOL_PING` / `PROTOCOL_LIST`（仅常量，与 C 服务端一致）。
- 请求 id 多路复用、daemon/pre-fork、graceful drain。

## 从 C API 过来应该看什么？

先读[快速上手](getting-started.md)与[客户端](client.md)，再看[README](../README.md)
里少量 Rust 命名对照。协议本身以 C 项目文档为准。
