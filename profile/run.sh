#!/usr/bin/env bash
# profile/run.sh — orchestrate one yar benchmark run (podman + profiler).
#
# One invocation does the whole k6-style cycle for one target:
#   1. verify the target image exists (print exact build commands if not),
#   2. check the port is free,
#   3. start the server container with parity resource limits,
#   4. wait for readiness (<= 10 s),
#   5. poll the server cgroup from the HOST every 200 ms
#      (cpu.stat usage_usec + memory.current; NOT podman exec, which would
#      run inside the container cgroup and pollute the numbers),
#   6. run the Rust profiler container against the server,
#   7. tear everything down (trap guarantees cleanup on any exit),
#   8. merge cgroup samples into the profiler CSV (cpu_pct / mem_peak) and
#      write report-<target>-<epoch>.csv plus an aligned summary table.
#
# Works from any CWD (script dir / repo root are resolved here).
#
# Exit codes: 0 ok, 1 runtime/verification failure, 2 usage error.
set -euo pipefail

usage() {
	cat >&2 <<'EOF'
Usage: run.sh --target <rust|c> [options]

Options:
  --target <rust|c>   Which server image to benchmark (required)
  --matrix <full|quick>  Stage matrix (default: full)
                        full  = profiler defaults: sizes 256b,1k,4k,16k,64k,
                                256k,512k,1m,2m,4m x vus 10,20,40,80,160,320,640
                        quick = sizes 1k,64k,1m,4m x vus 10,40,160,640
  --cpus <n>          --cpus for BOTH server and profiler containers (default: 2)
  --memory <size>     --memory / --memory-swap for both SERVERS (default: 2g)
  --inflight <n>      Server in-flight limit (default: 0 = unlimited).
                        Rust: --inflight N is a *connection* gate — a permit is
                        held for the whole connection, so with persistent VUs
                        only N VUs are ever served and the rest time out.
                        C: -n N event-loop workers; with 0 the C server keeps
                        its own default of 2 workers.
  --step-time <dur>   Per-stage profile duration, s/ms suffix (default: 10s)
  --out-dir <dir>     Output directory for the final report CSV (default: results)
  --port <n>          Server listen port via host network (default: 9501)
  -h, --help          Show this help
EOF
}

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
BUILD_HINT="(run from the repo root: $repo_root)"

RUST_IMG=localhost/yar-bench-rust:latest
C_IMG=localhost/yar-bench-c:latest

TARGET=""
MATRIX=full
CPUS=2
MEM=2g
INFLIGHT=0
STEP=10s
OUT_DIR=results
PORT=9501

while [ $# -gt 0 ]; do
	case $1 in
	--target | --matrix | --cpus | --memory | --inflight | --step-time | --out-dir | --port)
		if [ $# -lt 2 ]; then
			echo "run.sh: missing value for $1" >&2
			usage
			exit 2
		fi
		case $1 in
		--target) TARGET=$2 ;;
		--matrix) MATRIX=$2 ;;
		--cpus) CPUS=$2 ;;
		--memory) MEM=$2 ;;
		--inflight) INFLIGHT=$2 ;;
		--step-time) STEP=$2 ;;
		--out-dir) OUT_DIR=$2 ;;
		--port) PORT=$2 ;;
		esac
		shift 2
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		echo "run.sh: unknown argument: $1" >&2
		usage
		exit 2
		;;
	esac
done

die_usage() { echo "run.sh: $1" >&2; usage; exit 2; }

[ -n "$TARGET" ] || die_usage "--target is required (rust|c)"
case $TARGET in
rust | c) ;;
*) die_usage "--target must be 'rust' or 'c' (got '$TARGET')" ;;
esac
case $MATRIX in
full | quick) ;;
*) die_usage "--matrix must be 'full' or 'quick' (got '$MATRIX')" ;;
esac
case $INFLIGHT in
'' | *[!0-9]*) die_usage "--inflight must be a non-negative integer (got '$INFLIGHT')" ;;
esac
case $PORT in
'' | *[!0-9]*) die_usage "--port must be an integer (got '$PORT')" ;;
esac
[ "$PORT" -ge 1 ] && [ "$PORT" -le 65535 ] || die_usage "--port must be in 1..65535 (got '$PORT')"
case $STEP in
'' | [!0-9]*) die_usage "--step-time must be a duration with an s/ms suffix, e.g. 10s (got '$STEP')" ;;
esac

if [ "$MATRIX" = quick ]; then
	VUS="10,40,160,640"
	SIZES="1k,64k,1m,4m"
else
	# full = profiler's own defaults (src/profiler.rs DEFAULT_SIZES / DEFAULT_VUS).
	VUS="10,20,40,80,160,320,640"
	SIZES="256b,1k,4k,16k,64k,256k,512k,1m,2m,4m"
fi

print_build_help() {
	echo "run.sh: $BUILD_HINT" >&2
	if [ "$1" = rust ]; then
		echo "run.sh: missing image: localhost/yar-bench-rust:latest — build it with:" >&2
		echo "    podman build --http-proxy=false -f profile/Dockerfile -t yar-bench-rust ." >&2
	else
		echo "run.sh: missing image: localhost/yar-bench-c:latest — build it with:" >&2
		echo '    stage=$(profile/stage.sh); podman build --http-proxy=false -f profile/Dockerfile-yarc -t yar-bench-c "$stage"; rm -rf "$stage"' >&2
		if ! podman image exists "$RUST_IMG"; then
			echo "run.sh: also missing: localhost/yar-bench-rust:latest (carries the profiler) — build it with:" >&2
			echo "    podman build --http-proxy=false -f profile/Dockerfile -t yar-bench-rust ." >&2
		fi
	fi
}

if [ "$TARGET" = rust ]; then
	SRV_IMG=$RUST_IMG
	if ! podman image exists "$SRV_IMG"; then
		print_build_help rust
		exit 1
	fi
else
	SRV_IMG=$C_IMG
	if ! podman image exists "$SRV_IMG"; then
		print_build_help c
		exit 1
	fi
	if ! podman image exists "$RUST_IMG"; then
		print_build_help c
		exit 1
	fi
fi

# Sanity: nothing else may own the port. We never kill foreign processes —
# the operator resolves the conflict.
port_open() {
	(exec 3<>/dev/tcp/127.0.0.1/"$1") 2>/dev/null
}
if port_open "$PORT"; then
	echo "run.sh: port $PORT already in use on 127.0.0.1; free it or pass --port <n>" >&2
	exit 1
fi

mkdir -p "$OUT_DIR"
OUT_DIR=$(cd "$OUT_DIR" && pwd) # the bind mount needs an absolute path
rm -f "$OUT_DIR/.poll-abort"    # a stale marker from an aborted run must not fail this one

CG_CSV="$OUT_DIR/cgroup-$TARGET-$$.csv"
PROF_CSV="$OUT_DIR/profiler-$TARGET-$$.csv"
SRV="yar-bench-$TARGET-$$"
PROF="yar-prof-$$"
POLL_PID=""
CLEANED=""

cleanup() {
	if [ -n "$CLEANED" ]; then
		return
	fi
	CLEANED=1
	if [ -n "$POLL_PID" ]; then
		kill "$POLL_PID" 2>/dev/null || true
		wait "$POLL_PID" 2>/dev/null || true
		POLL_PID=""
	fi
	podman rm -f "$PROF" >/dev/null 2>&1 || true
	podman rm -f "$SRV" >/dev/null 2>&1 || true # ~10 s: server ignores SIGTERM
	SRV=""
}

trap cleanup EXIT
trap 'echo "run.sh: interrupted/failing; cleaning up" >&2; cleanup; exit 130' INT TERM

# --------------------------------------------------------------------------
# cgroup poller: runs on the HOST, reads the container's cgroup v2 files
# directly (podman exec would execute inside the container cgroup and skew
# the CPU numbers with exec-side helper processes).
# --------------------------------------------------------------------------
poller() {
	local pid cgv2 fails=0
	pid=$(podman inspect --format '{{.State.Pid}}' "$SRV" 2>/dev/null)
	cgv2=$(awk -F'::' '{print $2}' "/proc/$pid/cgroup")
	local path_cpu="/sys/fs/cgroup${cgv2}/cpu.stat"
	local path_mem="/sys/fs/cgroup${cgv2}/memory.current"
	while true; do
		local ts cpu mem
		ts=$(date +%s.%N)
		cpu=$(awk '$1=="usage_usec" { print $2; exit }' "$path_cpu" 2>/dev/null)
		mem=$(cat "$path_mem" 2>/dev/null)
		if [ -n "$cpu" ] && [ -n "$mem" ]; then
			printf '%s,%s,%s\n' "$ts" "$cpu" "$mem" >>"$CG_CSV"
			fails=0
		else
			fails=$((fails + 1))
			if [ "$fails" -ge 5 ]; then
				echo "run.sh: cgroup poll failed $fails consecutive times ($path_cpu | $path_mem)" >&2
				echo "run.sh: aborting run; intermediates kept in $OUT_DIR for debugging" >&2
				touch "$OUT_DIR/.poll-abort"
				kill -INT $$ # $$ = main shell pid, even inside a background subshell
				return 0
			fi
		fi
		sleep 0.2
	done
}

echo "[1/4] server: $SRV_IMG ($TARGET, inflight=$INFLIGHT, cpus=$CPUS, mem=$MEM)"
if [ "$TARGET" = rust ]; then
	# --max-request-size 8m (binary default): a full 4 MiB body + 82-byte
	# header + msgpack envelope is 4194333 bytes, which a 4m cap rejects
	# outright, so the mandated 4m stage would fail every request.
	SERVER_ARGS=(--host "0.0.0.0:$PORT" --inflight "$INFLIGHT" --max-request-size 8m --read-timeout 5s)
else
	# Worker-count peer: an explicit --inflight N is passed to both servers;
	# 0 means "unlimited connections" on the Rust side, whose C peer is the
	# C server's own default of 2 event-loop workers.
	C_WORKERS=2
	[ "$INFLIGHT" -gt 0 ] && C_WORKERS=$INFLIGHT
	SERVER_ARGS=(-S "0.0.0.0:$PORT" -n "$C_WORKERS" -l /tmp/yar-c.log)
fi
podman run -d \
	--name "$SRV" \
	--cpus="$CPUS" --memory="$MEM" --memory-swap="$MEM" \
	--network=host \
	--ulimit nofile=65536 \
	--http-proxy=false \
	"$SRV_IMG" "${SERVER_ARGS[@]}" >/dev/null

# Readiness gate: accept on 127.0.0.1:$PORT within 10 s, abort with logs if not.
ready=0
for i in $(seq 1 50); do
	if port_open "$PORT"; then
		ready=1
		break
	fi
	if [ "$(podman inspect --format '{{.State.Running}}' "$SRV" 2>/dev/null)" != "true" ]; then
		break
	fi
	sleep 0.2
done
if [ "$ready" != 1 ]; then
	echo "run.sh: server did not accept on 127.0.0.1:$PORT within 10s; container logs:" >&2
	podman logs "$SRV" 2>&1 || true
	exit 1
fi
echo "[2/4] server ready on 127.0.0.1:$PORT; host-side cgroup poller -> $CG_CSV"

poller &
POLL_PID=$!
sleep 0.3 # let the poller record a first sample before load starts

# --------------------------------------------------------------------------
# Load: the Rust profiler binary, containerized for identical host overhead.
# --------------------------------------------------------------------------
echo "[3/4] profiler: matrix=$MATRIX vus=$VUS sizes=$SIZES step=$STEP out=$PROF_CSV"
# The profiler itself holds several copies of the body per VU thread (request
# payload, response body, validation buffer), so its footprint scales with
# VUs x body size: at 4m x 640 VU it peaks at ~4.1 GB (measured) and a 2g or
# 4g cap gets it OOM-killed mid-run (exit 137, no CSV at all). Tuning the
# allocator (MALLOC_ARENA_MAX) is NOT an option: it halves the small-body
# throughput the profiler can generate and would silently skew every 256B-64k
# stage. The SERVERS above keep the mandated 2g; the load generator gets room.
PROF_MEM=6g
set +e
podman run --rm \
	--name "$PROF" \
	--cpus="$CPUS" --memory="$PROF_MEM" --memory-swap="$PROF_MEM" \
	--network=host \
	--ulimit nofile=65536 \
	--http-proxy=false \
	--entrypoint /usr/local/bin/profiler \
	-v "$OUT_DIR:/out:Z" \
	"$RUST_IMG" \
	--host "127.0.0.1:$PORT" \
	--vus "$VUS" --sizes "$SIZES" --step-time "$STEP" \
	--out "/out/profiler-$TARGET-$$.csv"
PROF_RC=$?
set -e

if [ -f "$OUT_DIR/.poll-abort" ]; then
	rm -f "$OUT_DIR/.poll-abort"
	echo "run.sh: aborted: cgroup sampling failed (see errors above)" >&2
	exit 1
fi

# Order per recipe: stop the poller, then the server, then merge.
if [ -n "$POLL_PID" ]; then
	kill "$POLL_PID" 2>/dev/null || true
	wait "$POLL_PID" 2>/dev/null || true
	POLL_PID=""
fi
cleanup # podman rm -f the server (also the profiler container if it lingered)

if [ "$PROF_RC" != 0 ]; then
	echo "run.sh: profiler exited with status $PROF_RC; intermediates kept in $OUT_DIR for debugging" >&2
	exit 1
fi
if [ ! -s "$PROF_CSV" ]; then
	echo "run.sh: profiler wrote no data to $PROF_CSV; intermediates kept in $OUT_DIR for debugging" >&2
	exit 1
fi

# --------------------------------------------------------------------------
# Merge: cgroup samples -> per-row cpu_pct / mem_peak. Both inputs are
# comma-separated (the poller writes its three columns with commas so a
# single -F',' serves both files; first_error may still contain commas,
# which is handled by counting epoch columns from the row end).
#
# Window per row = [start_ts, end_ts] (epoch seconds from the profiler CSV),
# including samples with ts >= start_ts && ts <= end_ts + 0.999 so the whole
# last second is covered.
#
#   cpu_pct  = (cpu_last - cpu_first) / (ts_last - ts_first) / 1e6 / CPUS * 100
#   mem_peak = max(memory.current) over the window
#
# If the window holds fewer than 2 samples, fall back to the nearest sample
# on each side; if there are no samples at all, emit 0/0 with a warning.
# --------------------------------------------------------------------------
REPORT="$OUT_DIR/report-$TARGET-$(date +%s).csv"
TMP_REPORT="$OUT_DIR/.report-$TARGET-$$.csv"

awk -F',' -v cpus="$CPUS" '
NR == FNR {
	n++
	ts[n] = $1 + 0
	cpu[n] = $2 + 0
	mem[n] = $3 + 0
	next
}
FNR == 1 {
	print "size_bytes,vus,duration_s,qps,ok,fail,p50_ms,p95_ms,p99_ms,p999_ms,first_error,start_ts,end_ts,cpu_pct,mem_peak"
	next
}
{
	# first_error (field 11) may contain commas, so count epoch columns
	# from the END of the row instead of trusting $12/$13.
	sh = NF - 13
	st = $(12 + sh) + 0
	et = $(13 + sh) + 0
	lim = et + 0.999
	fi = 0
	li = 0
	for (i = 1; i <= n; i++) {
		if (ts[i] >= st && ts[i] <= lim) {
			if (fi == 0) { fi = i }
			li = i
		}
	}
	first = 0
	last = 0
	if (fi != 0 && li - fi >= 1) {
		first = fi
		last = li
	} else if (n > 0) {
		bf = 0
		bl = 0
		bfd = 1e18
		afd = 1e18
		for (i = 1; i <= n; i++) {
			d = ts[i] - st; if (d < 0) { d = -d }
			if (d < bfd) { bfd = d; bf = i }
			d = ts[i] - et; if (d < 0) { d = -d }
			if (d < afd) { afd = d; bl = i }
		}
		first = bf
		last = bl
		if (first == last) {
			printf("run.sh merge: row %d: fewer than 2 samples in window (nearest collapses to one); cpu_pct=0\n", FNR) > "/dev/stderr"
		} else {
			printf("run.sh merge: row %d: fewer than 2 samples in window; using nearest samples\n", FNR) > "/dev/stderr"
		}
	} else {
		printf("run.sh merge: row %d: no cgroup samples available; cpu_pct=0, mem_peak=0\n", FNR) > "/dev/stderr"
	}
	cpu_pct = 0
	mem_peak = 0
	if (first != 0 && last != 0) {
		dt = ts[last] - ts[first]
		dc = cpu[last] - cpu[first]
		if (dt > 0 && dc >= 0) cpu_pct = dc / dt / 1e6 / cpus * 100
		lo = (first < last ? first : last)
		hi = (first < last ? last : first)
		for (i = lo; i <= hi; i++)
			if (mem[i] > mem_peak) mem_peak = mem[i]
	}
	printf "%s,%.1f,%d\n", $0, cpu_pct, mem_peak
}
' "$CG_CSV" "$PROF_CSV" >"$TMP_REPORT"

if [ ! -s "$TMP_REPORT" ]; then
	echo "run.sh: merge produced an empty report; intermediates kept in $OUT_DIR for debugging" >&2
	exit 1
fi
mv "$TMP_REPORT" "$REPORT"

# Only the merged report survives; raw samples are debug intermediates that
# get removed after a SUCCESSFUL merge (kept on failure for debugging).
rm -f "$PROF_CSV" "$CG_CSV"

echo "[4/4] done"
echo
echo "size      vus   qps         ok       fail  p50      p95      p99      p99.9    cpu%    memMB"
awk -F',' '
FNR == 1 {
	next
}
{
	# first_error (field 11) may contain commas; columns after it shift.
	sh = NF - 15
	printf "%-9s %-5s %-11s %-8s %-5s %-8s %-8s %-8s %-8s %-7s %.1f\n",
		$1, $2, $4, $5, $6, $(7 + sh), $(8 + sh), $(9 + sh), $(10 + sh), $(14 + sh), $(15 + sh) / 1048576
}
' "$REPORT"
echo
echo "report: $REPORT"
