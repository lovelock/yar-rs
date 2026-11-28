#!/bin/sh
# Stage a temporary build context for profile/Dockerfile-yarc.
#
# Copied into the stage:
#   - ../yar-c          (READ-ONLY C sources; never modified)
#   - yar_echo_server.c (profile/ call-site echo server)
#   - yar-entrypoint.sh (profile/ container entrypoint wrapper)
#
# Prints the stage directory path on stdout (and nothing else). The stage is
# NOT removed here — the caller cleans it up.
#
# yar-c source resolution (first match wins):
#   1. $YAR_C_DIR, if set
#   2. sibling of this repo (works when the worktree sits next to ../yar-c)
#   3. sibling of the main git worktree (worktrees live elsewhere)
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

if [ -n "${YAR_C_DIR:-}" ]; then
	yar_c_src="$YAR_C_DIR"
elif [ -d "$repo_root/../yar-c" ]; then
	yar_c_src="$repo_root/../yar-c"
else
	main=$(git -C "$repo_root" worktree list --porcelain \
		| sed -n 's/^worktree //p' | head -n 1)
	yar_c_src="$main/../yar-c"
fi

if [ ! -d "$yar_c_src" ]; then
	echo "stage.sh: cannot find yar-c sources (tried: $yar_c_src)" >&2
	exit 1
fi

stage=$(mktemp -d "${TMPDIR:-/tmp}/yarc-stage.XXXXXX")

# -a preserves original mtimes: the autotools maintainer rules regen
# aclocal.m4/Makefile.in when configure.ac looks newer (fresh copy breaks that),
# which would require the pinned aclocal-1.15 that EL10 does not ship.
cp -a "$yar_c_src" "$stage/yar-c"
cp "$script_dir/yar_echo_server.c" "$stage/"
cp "$script_dir/yar-entrypoint.sh" "$stage/"

# Benchmark-only change, applied to the STAGED copy (../yar-c is never edited):
# C emits one access-log line per request at OKEY and flushes it, while the
# Rust server has no access log. Gate that one call behind an env var so both
# servers run with access logging off by default; `run.sh --access-log` sets
# YAR_BENCH_ACCESS_LOG=1 to measure the logging cost separately.
if ! command -v perl >/dev/null 2>&1; then
	echo "stage.sh: perl is required to apply the benchmark access-log gate" >&2
	exit 1
fi
perl -0pi -e 's/\byar_server_log\(ctx\);/if (getenv("YAR_BENCH_ACCESS_LOG")) yar_server_log(ctx);/' \
	"$stage/yar-c/yar_server.c"
if ! grep -q 'YAR_BENCH_ACCESS_LOG' "$stage/yar-c/yar_server.c"; then
	echo "stage.sh: failed to apply the benchmark access-log gate" >&2
	exit 1
fi

printf '%s\n' "$stage"
