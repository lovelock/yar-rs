# Profiler: k6-style C vs Rust benchmark

`run.sh` measures one RPC path — `echo(p)` where the server returns the
request parameter tree verbatim — under an identical cgroup budget, over a
matrix of **body size × virtual users**. It is meant to put the original C
implementation (`../yar-c`) and this Rust rewrite side by side.

Both servers are real yar servers on the classic binary protocol; the load
generator is the Rust `profiler` binary (one persistent connection per VU).

## What a run produces

- `results/report-<target>-<epoch>.csv` — one row per stage with QPS,
  success/failure counts, latency percentiles, and the server container's
  CPU share and memory peak.
- the same data as an aligned table on stdout.

## Layout

| file | role |
|---|---|
| `run.sh` | orchestration: containers, host-side cgroup sampling, CSV merge |
| `Dockerfile` | `yar-bench-rust` — `yar-server-rust` (entrypoint) + `profiler` |
| `Dockerfile-yarc` | `yar-bench-c` — `../yar-c` + `yar_echo_server.c` |
| `stage.sh` | builds the C image's context (copy of `../yar-c`) in a temp dir |
| `yar_echo_server.c` | the C echo service (same semantics as the Rust one) |
| `yar-entrypoint.sh` | keeps the container alive after the C server daemonizes |

## Building the images

Both images are AlmaLinux-based multi-stage builds; `../yar-c` is never
modified — `stage.sh` copies it to a temporary directory.

```sh
# Rust image (also carries the load generator)
podman build -f profile/Dockerfile -t yar-bench-rust .

# C image (build context = staged copy of ../yar-c)
stage=$(profile/stage.sh)
podman build -f profile/Dockerfile-yarc -t yar-bench-c "$stage"
rm -rf "$stage"
```

`run.sh` refuses to start and prints these commands when an image is missing.

## Running

```sh
./profile/run.sh --target rust --matrix quick      # or --target c
```

| option | default | meaning |
|---|---|---|
| `--target rust\|c` | required | which server image to benchmark |
| `--matrix full\|quick` | `full` | stage matrix, see below |
| `--cpus <n>` | `2` | `--cpus` for both containers |
| `--memory <size>` | `2g` | server `--memory`/`--memory-swap` |
| `--inflight <n>` | `0` | server in-flight limit, `0` = unlimited (see parity) |
| `--step-time <dur>` | `10s` | duration of one stage (`s`/`ms`) |
| `--out-dir <dir>` | `results` | where the report is written |
| `--port <n>` | `9501` | listen port (`--network=host`) |

`--matrix quick` is the "did I break something" matrix:

- full: bodies `256b,1k,4k,16k,64k,256k,512k,1m,2m,4m` × VUs `10,20,40,80,160,320,640`
- quick: bodies `1k,64k,1m,4m` × VUs `10,40,160,640`

Every stage runs for `--step-time`, and there is a 1 s quiet gap between
stages during which the previous stage's persistent connections are closed
(each stage dials fresh connections).

## What happens during a run

1. The server container starts with the resource limits above.
2. Readiness is probed for up to 10 s.
3. The **host** polls the server container's cgroup v2 every 200 ms:
   `cpu.stat` (`usage_usec`) and `memory.current`. It deliberately does not
   use `podman exec`, whose helper processes would run inside the container
   cgroup and be charged to the server.
4. The profiler container starts `--entrypoint /usr/local/bin/profiler`
   with the out dir bind-mounted at `/out`.
5. Containers are torn down (`podman rm -f`; the C image needs SIGKILL,
   budget ~10 s) and the samples are merged into the report.

## Report columns

| column | meaning |
|---|---|
| `size_bytes` | echoed string length |
| `vus` | concurrent virtual users (one persistent connection each) |
| `duration_s` | configured stage length |
| `qps` | completed calls (`ok+fail`) per second |
| `ok`, `fail` | completed calls by outcome |
| `p50_ms` … `p999_ms` | latency percentiles of the **ok** samples |
| `first_error` | first failure text seen in the stage (empty when none) |
| `start_ts`, `end_ts` | stage window, epoch seconds |
| `cpu_pct` | container CPU as % of the `--cpus` quota (100 = one full CPU) |
| `mem_peak` | container `memory.current` peak during the stage, bytes |

A failure is any connection error, timeout, `status != 0`, or an echo that
does not come back byte-identical (length plus first/last 16 bytes).

## Resource parity and `--inflight`

Both servers get the same `--cpus`/`--memory` and the same listen port on
the host network; the load generator runs in a container too so it pays the
same syscall overhead on both sides.

Worker capacity is the one place the two designs genuinely differ:

- C: `-n N` forks `N` event-loop workers; each serves **many** connections
  concurrently.
- Rust: `max_inflight` is a **connection** gate (a permit is held for as
  long as the connection lives). With persistent VUs, `--inflight N` would
  therefore serve exactly `N` connections and starve the rest — for
  `N=2` and 640 VUs that is not a measurement, it is a queue.

So the default is `--inflight 0` (unlimited connections) for Rust, with the
C peer being its default `-n 2` workers: both then run 2 event-loop/task
workers' worth of parallelism on a 2-CPU quota. If you pass an explicit
`--inflight N`, the same `N` is passed to both servers (`-n N` /
`--inflight N`) — but note the caveat above: explicit small values will
starve the high-VU stages.

The profiler container itself gets more memory (`4g`). At `4m × 640` the
load generator holds a full copy of every body per VU and is OOM-killed
under 2 GB; the server under test keeps the mandated 2 GB.

## Sample results

One `--matrix quick` run per target on the same machine (2 CPU / 2 GB per
container, `--network=host`, 10 s per stage, Rust server `--inflight 0`,
C server `-n 2`). Both runs completed with no leaked containers.

| body | VUs | Rust QPS | C QPS | Rust p99 ms | C p99 ms | Rust cpu% | C cpu% | Rust mem | C mem | Rust fail | C fail |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1k | 10 | 183,721 | 99,408 | 0.09 | 0.15 | 66 | 84 | 3M | 99M | 0 | 0 |
| 1k | 40 | 154,685 | 93,485 | 0.33 | 0.49 | 56 | 77 | 3M | 189M | 0 | 0 |
| 1k | 160 | 154,986 | 96,047 | 2.31 | 2.19 | 59 | 82 | 4M | 282M | 0 | 0 |
| 1k | 640 | 156,084 | 93,116 | 24.34 | 8.28 | 62 | 79 | 8M | 379M | 0 | 0 |
| 64k | 10 | 25,113 | 15,332 | 0.64 | 0.93 | 84 | 89 | 4M | 391M | 0 | 0 |
| 64k | 40 | 25,087 | 14,162 | 2.44 | 3.58 | 86 | 88 | 6M | 410M | 0 | 0 |
| 64k | 160 | 22,321 | 11,064 | 11.20 | 16.09 | 89 | 85 | 14M | 441M | 0 | 0 |
| 64k | 640 | 22,941 | 11,940 | 60.11 | 58.21 | 85 | 87 | 49M | 537M | 0 | 0 |
| 1m | 10 | 1,317 | 734 | 12.83 | 19.83 | 86 | 87 | 29M | 467M | 0 | 0 |
| 1m | 40 | 375 | 460 | 206.76 | 194.63 | 24 | 49 | 60M | 562M | 0 | 0 |
| 1m | 160 | 329 | 372 | 1702.06 | 1518.15 | 20 | 39 | 82M | 554M | 0 | 2 |
| 1m | 640 | 367 | 363 | 4099.33 | 3193.50 | 18 | 30 | 123M | 539M | 537 | 342 |
| 4m | 10 | 82 | 90 | 203.10 | 202.32 | 29 | 38 | 144M | 563M | 0 | 0 |
| 4m | 40 | 55 | 58 | 1406.91 | 1606.14 | 16 | 19 | 189M | 717M | 0 | 0 |
| 4m | 160 | 64 | 74 | 5714.79 | 4145.37 | 13 | 13 | 437M | 847M | 39 | 272 |
| 4m | 640 | 66 | 85 | 6713.21 | 6351.66 | 14 | 11 | 482M | 887M | 37 | 402 |

Reading the table:

- **Small bodies (≤64k):** Rust is ~1.6–2.2× the C QPS at every VU level —
  the C server pays more per-request overhead (unpack/zone/pack round trip).
- **Large bodies (≥1m):** the two converge; C is slightly ahead at 4m, where
  the Rust path copies the body through the packager.
- **Memory:** C's pre-forked workers hold per-connection buffers and stay in
  the hundreds of MB; Rust's footprint tracks the in-flight bodies (and the
  probe at 4m×640 stays under the 2 GB cap).
- **CPU:** at ≥1m the server is not the bottleneck — the 2-CPU load generator
  cannot feed 160–640 clients with multi-MB bodies fast enough, which is why
  `cpu%` drops to 10–30 % while QPS flattens.
- **Top-right corner:** `1m×640`, `4m×160`, `4m×640` show `Broken pipe`
  failures on both sides. The server closes connections that stay idle for
  its 5 s read timeout while the starved load generator is still producing
  the request; this is a limit of the 2-CPU measuring rig, not of the RPC
  path. The full matrix shows the same rows with `--step-time 10s`; treat
  them as "not measurable here" rather than as a server result.
- These are single runs on a shared workstation (a few percent run-to-run
  variance, more at the 1m/4m corners). Re-run `--matrix full` for a real
  curve.

