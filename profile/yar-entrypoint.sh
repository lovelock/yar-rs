#!/bin/sh
# Container entrypoint for the C yar benchmark image (yar-bench-c).
#
# Launch the C yar server (daemonizes in non-standalone mode), then keep
# the container's PID 1 alive because the daemonizer's parent exits.
/usr/local/bin/yar-server-c "$@"
rc=$?
if [ "$rc" -ne 0 ]; then exit "$rc"; fi
exec sleep infinity
