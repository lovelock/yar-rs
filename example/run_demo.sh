#!/bin/bash
#
# Cross-language interop demo for yar C <-> Rust
#
# Demo 1: C Server + Rust Client
# Demo 2: Rust Server + C Client
#
# Usage:
#   ./run_demo.sh          # run both demos
#   ./run_demo.sh 1        # run demo 1 only (C server, Rust client)
#   ./run_demo.sh 2        # run demo 2 only (Rust server, C client)

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

PORT1=19501
PORT2=19502
HOST="127.0.0.1"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

banner() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║          $1${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

info() {
    echo -e "${YELLOW}[INFO]${NC} $1"
}

ok() {
    echo -e "${GREEN}[OK]${NC} $1"
}

fail() {
    echo -e "${RED}[FAIL]${NC} $1"
}

# Build everything
build() {
    banner "Building C examples..."
    cd "$SCRIPT_DIR"
    make clean 2>/dev/null || true
    make all
    ok "C examples built"

    banner "Building Rust examples..."
    cd "$PROJECT_DIR"
    cargo build --examples 2>&1 | tail -5
    ok "Rust examples built"
}

# Wait for a server to be ready by checking TCP port
wait_for_port() {
    local host=$1
    local port=$2
    local max_wait=$3
    local waited=0
    while ! nc -z "$host" "$port" 2>/dev/null; do
        sleep 0.1
        waited=$((waited + 1))
        if [ "$waited" -ge "$((max_wait * 10))" ]; then
            fail "Server on $host:$port did not start within ${max_wait}s"
            return 1
        fi
    done
    return 0
}

# ============================================================
# Demo 1: C Server + Rust Client
# ============================================================
demo1() {
    banner "Demo 1: C Server ← Rust Client"
    info "Starting C server on ${HOST}:${PORT1}..."

    cd "$SCRIPT_DIR"
    ./cross_server -S "${HOST}:${PORT1}" -X &
    SERVER_PID=$!

    if ! wait_for_port "$HOST" "$PORT1" 10; then
        kill $SERVER_PID 2>/dev/null
        return 1
    fi
    ok "C server started (pid=$SERVER_PID)"

    info "Running Rust client against C server..."
    echo -e "${CYAN}---${NC}"
    cd "$PROJECT_DIR"
    if cargo run --example cross_client -- -h "${HOST}:${PORT1}" -n 2 2>&1; then
        ok "Demo 1 PASSED: Rust client successfully communicated with C server"
    else
        fail "Demo 1 FAILED"
    fi
    echo -e "${CYAN}---${NC}"

    kill $SERVER_PID 2>/dev/null
    wait $SERVER_PID 2>/dev/null
    info "C server stopped"
}

# ============================================================
# Demo 2: Rust Server + C Client
# ============================================================
demo2() {
    banner "Demo 2: Rust Server ← C Client"
    info "Starting Rust server on ${HOST}:${PORT2}..."

    cd "$PROJECT_DIR"
    cargo run --example cross_server -- -S "${HOST}:${PORT2}" &
    SERVER_PID=$!

    if ! wait_for_port "$HOST" "$PORT2" 10; then
        kill $SERVER_PID 2>/dev/null
        return 1
    fi
    ok "Rust server started (pid=$SERVER_PID)"

    info "Running C client against Rust server..."
    echo -e "${CYAN}---${NC}"
    cd "$SCRIPT_DIR"
    if ./cross_client -h "${HOST}:${PORT2}" -n 2; then
        ok "Demo 2 PASSED: C client successfully communicated with Rust server"
    else
        fail "Demo 2 FAILED"
    fi
    echo -e "${CYAN}---${NC}"

    kill $SERVER_PID 2>/dev/null
    wait $SERVER_PID 2>/dev/null
    info "Rust server stopped"
}

# ============================================================
# Main
# ============================================================

echo -e "${CYAN}"
echo "  ╔═══════════════════════════════════════════╗"
echo "  ║   yar C <-> Rust Cross-Language Demo     ║"
echo "  ╚═══════════════════════════════════════════╝"
echo -e "${NC}"

build

case "${1:-all}" in
    1)
        demo1
        ;;
    2)
        demo2
        ;;
    *)
        demo1
        echo ""
        sleep 1
        demo2
        ;;
esac

banner "All demos complete!"
