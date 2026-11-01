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

printf '%s\n' "$stage"
