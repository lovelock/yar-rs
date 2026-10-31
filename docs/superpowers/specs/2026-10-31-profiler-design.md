# 设计：k6 式 profiler（C 版 ~ Rust 版 对比基准）

日期：2026-10-31
状态：待批准

## 意图与动机

项目已将 Yar 改写为 Rust（yar-rs），需要量化它与原 C 实现
（`../yar-c`，宿主机上保存副本）在相同资源约束下的性能差异，
并顺带让 Rust 版具备与 C 版对等的资源控制能力。

产出是一个 k6 风格的基准报告：对每个 `(body size × VU)` 组合
输出 QPS、成功/失败计数、p50/p95/p99/p99.9 延迟，以及服务端
容器 cgroup 采集的 CPU 与内存峰值。

## 目标

1. 两个 podman 镜像，各 2C/2G、`--network=host`，基座统一用
   `docker.io/library/almalinux:latest`（AlmaLinux 9，已拉取）：
   - `Dockerfile`（rust 镜像）：多阶段（builder 与 runtime 都基于
     almalinux；builder 经 dnf 安装编译工具链（`rust-toolset`，
     版本不足时退化为 rustup），release 构建 yar-rs，内含
     `yar-server-rust`（echo 服务）与 `profiler`（压测客户端）两个入口；
   - `Dockerfile-yarc`：build context 为 run.sh 生成的暂存目录
     （含 `../yar-c` 与自实现的 `yar_echo_server.c`），镜像内编译
     libevent + msgpack-c + echo server。
2. 只测一个接口：`echo(p: [bytes…])` → 服务端把 `p` 原样放回 `r`。
3. 压测模型：
   - 每个 VU 一条**持久连接**（对应 `YarClient::set_persistent`，
     等价 k6 每连接一 VU 的模型）；
   - VU 阶梯默认 `10,20,40,80,160,320,640`（CLI 可覆盖），
     每档默认时长 10s（CLI 可覆盖）；
   - body 阶梯采用更细的密度（约每档 4 倍递增，低端更密）：
     `256B, 1K, 4K, 16K, 64K, 256K, 512K, 1M, 2M, 4M`
     （CLI 可覆盖；256B~64K 覆盖 RPC 常见负载区，1M~4M 覆盖吞吐饱和区）；
     全矩阵耗时较长：10 档 body × 7 档 VU × 10s ≈ 12 分钟/目标，
     推荐单次运行用 `--sizes`/`--vus` 取子集，run.sh 提供
     `--matrix full|quick`（quick 仅取 body 1K,64K,1M,4M × VU 间隔档）；
   - 两端服务端都开启 4M 请求上限校验；超 4M 的档不存在
     （body 上限恒为 4M）。
4. 报告：CSV + 终端摘要表；字段包括
   `size, vus, qps, ok, fail, p50, p95, p99, p99.9, cpu_pct(相对 2C 配额),
    mem_peak(容器 cgroup memory.current 档内峰值)`。
5. 服务端资源对齐：
   - C 版 pre-fork worker 数与 Rust 版 `max_inflight` 使用**同一个数**，
     默认 2（与 2C 配额匹配），由 run.sh 统一传参；
   - 两端 server read timeout 一律 5s（Rust 版由 profiler/entrypoint 设置）；
   - C 版 echo handler 使用 yar 原生 unpack/zone/pack 流程，不人为
     改变内存行为；两端内存差异由 cgroup 实测反映。
6. Rust 库内新增控制项（真实实现，移除占位语义）：
   - `YarServer::set_max_inflight(n: usize)`：用 `tokio::sync::Semaphore`
     限制同时处理的连接数（超出排队）。默认无上限（保持现行为），
     profiler 显式传入与 C 相同的 worker 数；
   - `YarServer::set_max_request_size(bytes: usize)`：解析完 header 后
     先校验 `body_len`，超限回 status != 0 错误响应并关闭连接，
     从而在读 body 前止损。默认 8 MiB（覆盖 4M body + 协议开销余量）。
   - 两项都带单元测试；不改变默认行为时现有测试全部保持通过。

## 非目标

- 不测 HTTP 传输、PHP 服务端互通；
- 不做分布式压测（单机双容器）；
- 不做 perf 火焰图/符号级 CPU profile（后续如有需要另行设计）；
- 不实现 k6 全部概念（阈值、场景脚本等）。

## 约束

- profiler 基于 `YarClient`（阻塞 API）：VU 线程 + `spawn_blocking`
  组织并发；不引入额外客户端实现；
- 不修改 `../yar-c` 的任何文件——echo 源码放在本仓库
  `profile/yarc/yar_echo_server.c`，由 run.sh 暂存进构建上下文
  （拷贝到临时目录随 yar-c 一起 build），保持宿主目录干净；
- `--network=host` 避免 NAT 引入的延迟畸变；
- 压测期间容器资源由 podman 的 cgroup 参数施加
  （`--cpus=2 --memory=2g`），数值由 run.sh 统一定义，两边一致。

## 结构

```text
yar-rs/
├── src/
│   ├── server.rs            # + max_inflight / max_request_size（实现+测试）
│   └── bin/profiler.rs      # 压测器（workspace 内新 bin，无新依赖）
├── profile/
│   ├── Dockerfile           # almalinux 基座：server + profiler 双入口镜像
│   ├── Dockerfile-yarc      # almalinux 基座：C 镜像（context=run.sh 暂存目录）
│   ├── yar_echo_server.c    # C echo server 源码（基于 example/server.c 改造）
│   └── run.sh               # 起 N 个容器、轮询 cgroup、汇总报告
└── docs/superpowers/specs/2026-10-31-profiler-design.md   # 本文档
```

## profiler CLI 草案

```text
profiler --target rust|c --host 127.0.0.1:9501
         --vus 10,20,40,80,160,320,640
         --sizes 256b,1k,4k,16k,64k,256k,512k,1m,2m,4m
         --step-time 10s
         --inflight 2            # 传给 rust 服务端的 max_inflight
         --children  2           # 传给 C 服务端的 YAR_MAX_CHILDREN
         --read-timeout 5s
         --body 4m               # 服务端请求上限（两端一致）
         --out report.csv
```

琥珀细节（验收要点）：

- 4M body × 高 VU 时内存峰值可能先于 QPS 见顶；报告保留该现象并
  通过 cpu/mem 列可见，不以"自动降压"掩盖；
- 阶梯切换期间先静默 1s（让上一档连接完全收尾，避免失真）；
- 每档开始后延迟样本只统计该档时间窗内的完整往返；
- 失败判定：连接错误、超时、status != 0、魔数错误全部计 fail；
  记录首个失败原因样本（防止日志刷屏）。

## 衡量成功

1. `cargo test` 保持全绿（含新增 max_inflight/max_request_size 测试）；
2. `profile/run.sh --target rust` 与 `--target c` 可用相同产品完成全矩阵，
   生成可比较的 CSV；
3. 两条路径（rust 服务的 echo、C 服务的 echo）在相同 2C2G 下给出的
   表格能直接并排阅读；

## 风险与已知取舍

- C 版 pre-fork 模型与 Rust 的连接并发模型并非逐位等价
  （C 的 child 是阻塞进程级并发、Rust 是 task 级），以"worker 容量
  相同值"作为公平化近似，报告中记录该配置；
- 4M body 高并发时 RSS 峰值可能触发 2G cgroup OOM——这正是要观察
  的性质；报告 fail 计数与 mem_peak 会呈现它；
- VU 极值（>1k）可能触及 fd 上限：run.sh 起容器前调
  `ulimit -n`/podman `--ulimit nofile=65536`。
