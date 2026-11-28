# Profiler: C vs Rust benchmark

`run.sh` measures one RPC path — `echo(p)` where the server returns the
request parameter tree verbatim — under an identical cgroup budget, over a
matrix of **body size × virtual users**. It puts upstream yar-c (`../yar-c`)
and this Rust rewrite side by side.

Both servers are real yar servers on the classic binary protocol; the load
generator is the Rust `profiler` binary (one persistent connection per VU).

## Parity rules

A run is only meaningful if both sides pay the same costs. `run.sh` enforces:

| item | C | Rust |
|---|---|---|
| CPU / memory cap | `--cpus`, `--memory` | same |
| network | `--network=host` | same |
| read timeout | `-t 3` (`YAR_READ_TIMEOUT`) | `--read-timeout 3s` |
| request cap | 10 MiB (`YAR_MAX_BODY_SIZE`) | `--max-request-size 10m` |
| worker parallelism | `-n 2` event-loop workers | `TOKIO_WORKER_THREADS=2` |
| access log | **off by default** (benchmark build gates the OKEY line behind `YAR_BENCH_ACCESS_LOG`) | `--no-access-log` |
| load generator | same `profiler` binary | same |

The access-log gate is the only change applied to yar-c, and it is applied to
the **staged copy** in `stage.sh` — `../yar-c` is never edited. C emits one
OKEY line per request and `fflush`es it; leaving that on while the Rust side
has no access log made every earlier comparison measure log I/O as much as
RPC. `--access-log` runs both sides with logging on if you want to measure
that cost explicitly.

## What a run produces

- `results/report-<target>-<epoch>.csv` — one row per stage with attempt
  QPS, **successful QPS**, ok/fail counts, latency percentiles, and the CPU
  / memory / throttling / generator-CPU columns.
- the same data as an aligned table on stdout.
- a warning on stderr if any stage's cgroup window has too few samples.

## Layout

| file | role |
|---|---|
| `run.sh` | orchestration: containers, host-side cgroup sampling, CSV merge |
| `Dockerfile` | `yar-bench-rust` — `yar-server-rust` (entrypoint) + `profiler` |
| `Dockerfile-yarc` | `yar-bench-c` — staged `../yar-c` + `yar_echo_server.c` |
| `stage.sh` | builds the C image's context and applies the access-log gate |
| `yar_echo_server.c` | the C echo service (same semantics as the Rust one) |
| `yar-entrypoint.sh` | keeps the container alive after the C server daemonizes |

## Building the images

```sh
podman build --http-proxy=false -f profile/Dockerfile -t yar-bench-rust .

stage=$(profile/stage.sh)
podman build --http-proxy=false -f profile/Dockerfile-yarc -t yar-bench-c "$stage"
rm -rf "$stage"
```

`run.sh` refuses to start and prints these commands when an image is missing.

## Running

```sh
./profile/run.sh --target rust --matrix quick
./profile/run.sh --target c    --matrix quick
```

| option | default | meaning |
|---|---|---|
| `--target rust\|c` | required | which server image to benchmark |
| `--matrix full\|quick` | `full` | stage matrix, see below |
| `--cpus <n>` | `2` | `--cpus` for the server container |
| `--client-cpus <n>` | `--cpus` | `--cpus` for the profiler container; give the generator headroom to prove a stage is not generator-bound |
| `--memory <size>` | `2g` | server `--memory`/`--memory-swap` |
| `--inflight <n>` | `0` | server in-flight limit, `0` = unlimited |
| `--step-time <dur>` | `10s` | measured window per stage |
| `--warmup <dur>` | `2s` | warmup before the measured window |
| `--access-log` | off | run both servers with the per-request access log on |
| `--sizes <csv>` | matrix | override the body ladder |
| `--vus <csv>` | matrix | override the VU ladder |
| `--out-dir <dir>` | `results` | where the report is written |
| `--port <n>` | `9501` | listen port (`--network=host`) |

`--matrix quick` is the "did I break something" matrix:

- full: bodies `0,256b,1k,4k,16k,64k,256k,512k,1m,2m,4m` × VUs `10,20,40,80,160,320,640`
- quick: bodies `0,1k,64k` × VUs `10,40,160,640` (small-body smoke)

Large bodies are run separately (see the memory note below):

```sh
./profile/run.sh --target rust --sizes 1m --vus 10,40,160 --client-cpus 4
./profile/run.sh --target rust --sizes 4m --vus 10,40     --client-cpus 4
```

`size_bytes = 0` sends an empty parameter string: the pure RPC-overhead row
(header + envelope + echo, no body copy to speak of).

## What happens during a run

1. The C image is built from a staged copy with the access-log gate; the Rust
   image carries both the server and the load generator.
2. The server container starts with the resource limits above.
3. Readiness is probed for up to 10 s.
4. The **host** polls both containers' cgroup v2 every 200 ms:
   `cpu.stat` (`usage_usec`, `nr_throttled`) and `memory.current`. It does not
   use `podman exec`, whose helper processes would run inside the container
   cgroup and be charged to the server.
5. The profiler container runs `/usr/local/bin/profiler` with the out dir
   bind-mounted at `/out`.
6. Containers are torn down and the samples are merged into the report.

## Report columns

| column | meaning |
|---|---|
| `size_bytes` | echoed string length |
| `vus` | concurrent virtual users (one persistent connection each) |
| `duration_s` | configured measured window |
| `qps` | completed calls (`ok+fail`) per second — attempts, not throughput |
| `ok_qps` | **successful** calls per second — the metric to compare |
| `ok`, `fail` | completed calls by outcome |
| `p50_ms` … `p999_ms` | latency percentiles of the **ok** samples |
| `first_error` | first failure text seen in the stage (empty when none) |
| `start_ts`, `end_ts` | measured window in fractional epoch seconds |
| `cpu_pct` | server container CPU as % of the `--cpus` quota (with `--cpus=2`, **100 = both cores**) |
| `mem_peak` | server container `memory.current` peak during the window, bytes |
| `client_cpu_pct` | profiler container CPU as % of `--client-cpus`; `-1` when unsampled |
| `srv_throttled` | delta of the server cgroup's `nr_throttled` over the window |

A failure is any connection error, timeout, `status != 0`, or an echo that
does not come back byte-identical (length plus first/last 16 bytes).

## Why `ok_qps` and `client_cpu_pct`

- `qps` counts failed sends too. A "Broken pipe" after a server-side idle
  timeout costs almost nothing and would inflate the number; in the old 4m
  rows, about half of C's "completed calls" were failures. Compare `ok_qps`.
- The generator used to repack the request and rebuild the response tree on
  every call (about 8 full-body copies per call). It now builds the request
  packet once per stage and reuses it via `call_raw`, so a 2-vCPU generator
  can actually drive the server. If `client_cpu_pct` is near 100, the stage
  is generator-bound and says nothing about the server; give the generator
  more CPU with `--client-cpus` and rerun.
- The measured window starts only after every VU has connected (barrier) and
  warmed up, so the 640-thread connection ramp is not counted.

## What is still not measured

- Non-persistent (one connection per call) workloads.
- Open-loop / constant-arrival-rate saturation points; this is closed-loop.
- Multiple repeats and A/B interleaving; run each target 3+ times and compare
  medians.
- Server-side attribution (`perf stat/record` cycles, instructions, syscalls
  per request). The report is end-to-end.

**Memory-bound cells.** Every in-flight request needs at least one body
buffer on the server. At 4 MiB × 640 VUs that is 2.5 GiB before the request
tree, the packet builders or the response; observed server peak exceeds
4 GiB and the container is OOM-killed (the cgroup disappears and the poller
aborts the run). Do not put `4m × 640` in a 2 GiB matrix; select a feasible
subset with `--sizes` / `--vus` (the sample matrix below uses
`0,1k,64k × 10..640`, `1m × 10..160`, `4m × 10,40`).

Treat the quick matrix as a smoke test; use `--matrix full`, `--client-cpus`
above `--cpus`, and several runs before drawing conclusions.

## Sample results

One curated quick matrix per target on a 12-core workstation: 2 vCPU / 2 GiB
per server, 4 vCPU for the generator (`--client-cpus 4`), 10 s measured
window, 2 s warmup, read timeout 3 s on both sides, access logging off on
both sides. The Rust rows are from this implementation; the C rows are from
the original `yar-c` at `1c56d90`, built from source with the same echo
service. `4m × 640` is omitted because it is memory-bound (see above).
Full rows: [`results/report-rust-quick.csv`](results/report-rust-quick.csv),
[`results/report-c-quick.csv`](results/report-c-quick.csv).

| bytes | VUs | Rust ok_qps | C ok_qps | Rust/C | Rust p99 ms | C p99 ms | Rust srv% | C srv% | Rust cli% | C cli% | Rust MiB | C MiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 10 | 226,103 | 189,406 | **1.19** | 0.09 | 0.09 | 76 | 83 | 42 | 34 | 6 | 10 |
| 0 | 40 | 241,062 | 182,685 | **1.32** | 0.29 | 0.37 | 76 | 82 | 47 | 40 | 6 | 9 |
| 0 | 160 | 241,171 | 181,282 | **1.33** | 1.36 | 1.27 | 75 | 82 | 49 | 40 | 7 | 10 |
| 0 | 640 | 240,588 | 170,459 | **1.41** | 13.33 | 5.85 | 76 | 81 | 55 | 42 | 12 | 14 |
| 1k | 10 | 211,025 | 172,974 | **1.22** | 0.09 | 0.10 | 77 | 82 | 42 | 34 | 6 | 10 |
| 1k | 40 | 214,049 | 167,311 | **1.28** | 0.31 | 0.33 | 76 | 81 | 47 | 41 | 6 | 10 |
| 1k | 160 | 225,768 | 170,752 | **1.32** | 1.07 | 1.42 | 78 | 83 | 51 | 38 | 7 | 11 |
| 1k | 640 | 198,283 | 146,912 | **1.35** | 17.53 | 6.74 | 75 | 80 | 51 | 41 | 12 | 16 |
| 64k | 10 | 33,763 | 34,743 | 0.97 | 0.51 | 0.89 | 91 | 90 | 26 | 28 | 8 | 13 |
| 64k | 40 | 32,609 | 18,320 | **1.78** | 2.15 | 3.67 | 91 | 94 | 28 | 19 | 10 | 18 |
| 64k | 160 | 29,414 | 13,129 | **2.24** | 8.95 | 17.86 | 90 | 92 | 34 | 19 | 19 | 44 |
| 64k | 640 | 30,183 | 12,140 | **2.49** | 33.45 | 66.44 | 90 | 94 | 38 | 19 | 53 | 134 |
| 1m | 10 | 2,431 | 1,314 | **1.85** | 7.21 | 12.54 | 94 | 94 | 53 | 31 | 34 | 48 |
| 1m | 40 | 2,328 | 1,097 | **2.12** | 28.39 | 54.69 | 94 | 96 | 57 | 28 | 75 | 186 |
| 1m | 160 | 2,339 | 1,096 | **2.13** | 106.31 | 239.14 | 95 | 96 | 56 | 27 | 301 | 638 |
| 4m | 10 | 312 | 285 | **1.10** | 63.47 | 55.13 | 96 | 97 | 39 | 36 | 186 | 180 |
| 4m | 40 | 310 | 279 | **1.11** | 272.14 | 258.69 | 97 | 97 | 38 | 43 | 495 | 611 |

Reading it:

- **≤1 KiB:** Rust is 1.19–1.41× ahead and uses less server CPU per request
  (6.7–7.3 µs vs 8.8–9.5 µs). The earlier "1.6–1.9×" was inflated by C's
  per-request access logging and by a generator that was itself saturated;
  both are fixed here.
- **64 KiB:** at 10 VUs C is ~3 % faster (34.7k vs 33.8k qps; ~51.8 µs vs
  53.7 µs of server CPU). At 40–640 VUs the C side's per-request cost rises
  to 87–146 µs while Rust stays at 54–60 µs, so Rust is 1.8–2.5× ahead and
  its p99 is roughly half. This is a scaling effect of the two event-loop
  workers, not a constant-factor win.
- **1 MiB:** both servers are CPU-saturated (94–96 %); Rust is 1.85–2.13×
  ahead, with p99 7/28/106 ms versus C's 13/55/239 ms.
- **4 MiB:** both are at ~96 % of the 2-vCPU quota; Rust is 1.10–1.11× ahead
  on throughput but C has the slightly better p99 (55/259 ms vs 63/272 ms).
  The 4 MiB rows are the second of two back-to-back runs: the first pair
  measured C 237/239 and Rust 319/320 qps, so treat this size as near parity
  with high run-to-run variance.
- **Memory:** Rust is 1.3–2.5× lower in the small/medium cells (e.g. 53 MiB
  vs 134 MiB at 64 KiB × 640, 301 MiB vs 638 MiB at 1 MiB × 160); at 4 MiB
  the two are comparable (186 vs 180 MiB at 10 VUs, 495 vs 611 MiB at 40).
  The old 342 MB C / 9 MB Rust row was dominated by the access-log file's
  page cache charged to the container cgroup.
- **Generator:** `cli%` stays at 19–57 % of the 4-vCPU generator quota, so
  these rows are server-bound (the old 2-vCPU generator was the bottleneck
  at ≥1 MiB, which is why QPS flattened and failures appeared).
- **CPU%** is percent of the 2-vCPU quota: 100 = both cores. CFS throttling
  was zero in every row.
- These are single runs (except the 4 MiB pair). Re-run each target 3+ times
  and compare medians before quoting a number outside this page.
