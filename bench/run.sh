#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-3.0-or-later
# Builds the benchmark programs against the current must and runs a suite (Linux).
# usage: bench/run.sh [polling|stall|throughput|all]   (default: polling)
set -euo pipefail

suite=${1:-polling}
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
out="$here/target"
port=21500

echo "== building" >&2
cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p must-ffi
cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p must-server --example simple_node
mkdir -p "$out/lib"
cp "$root/target/release/libmust_ffi.so" "$out/lib/libSHALLserver64.so"
cc -O2 -std=c99 -Wall -Wextra -I"$root/include" "$here/bench_host.c" \
    -L"$out/lib" -lSHALLserver64 -Wl,-rpath,"$out/lib" -o "$out/bench_host"
cargo build --quiet --release --manifest-path "$here/loadgen/Cargo.toml" --target-dir "$out/loadgen"

host="$out/bench_host"
load="$out/loadgen/release/loadgen"
node="$root/target/release/examples/simple_node"
export MUST_TUI=0 MUST_LOG_DIR="$out/logs"

server=
stop() {
    if [ -n "$server" ]; then
        kill "$server" 2>/dev/null || true
        wait "$server" 2>/dev/null || true
        server=
    fi
}
trap stop EXIT

# starts a server in the background (`env` arguments allowed) and waits for its port
start() {
    port=$((port + 1))
    env "$@" >/dev/null 2>&1 &
    server=$!
    for _ in $(seq 100); do
        if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then return; fi
        sleep 0.05
    done
    echo "server on port $port did not start" >&2
    exit 1
}

polling() {
    for style in drain one; do
        echo "== polling mode, 50 ms loop, GET CMD $([ $style = drain ] && echo 'until no data' || echo 'once per pass')"
        start BENCH_POLL=0 "$host" poll $((port + 1)) 50 $style
        a="127.0.0.1:$port"
        printf '%-40s' "1 client, one read at a time:"; "$load" read "$a" 1 1 3 "read m:p0"
        printf '%-40s' "4 clients, different parameters:"; "$load" read "$a" 4 1 3 "read m:p0" "read m:p1" "read m:p2" "read m:p3"
        printf '%-40s' "8 clients, same parameter:"; "$load" read "$a" 8 1 3 "read m:p0"
        printf '%-40s' "1 client, same read twice in a row:"; "$load" read "$a" 1 2 3 "read m:p0"
        stop
    done
}

stall() {
    echo "== polling mode, 50 ms loop, one client reading for 75 s (covers one 60 s slow-tier tick)"
    for poll in default 0; do
        if [ $poll = default ]; then start "$host" poll $((port + 1)) 50; else start BENCH_POLL=0 "$host" poll $((port + 1)) 50; fi
        printf '%-40s' "pollinterval $poll:"; "$load" read "127.0.0.1:$port" 1 1 75 "read m:p0"
        stop
    done
}

throughput() {
    for env in "MUST_LOG_FILES=1" "MUST_LOG_FILES=0 MUST_MONITOR=0"; do
        echo "== throughput, $env"
        # shellcheck disable=SC2086
        start $env "$node" "127.0.0.1:$((port + 1))"
        printf '%-40s' "Rust node, 16 clients x 8 in flight:"; "$load" read "127.0.0.1:$port" 16 8 3 "read cryo:value" "read cryo:target"
        stop
        # shellcheck disable=SC2086
        start $env "$host" cb $((port + 1))
        printf '%-40s' "C callbacks, 1 client x 1:"; "$load" read "127.0.0.1:$port" 1 1 3 "read m:p0"
        printf '%-40s' "C callbacks, 16 clients x 8 in flight:"; "$load" read "127.0.0.1:$port" 16 8 3 "read m:p0" "read m:p1"
        stop
        port=$((port + 1))
        printf '%-40s' "SECoP_S_updateParameter2 loop:"
        # shellcheck disable=SC2086
        env $env "$host" push "$port" 3 2>/dev/null
    done
}

case "$suite" in
    polling) polling ;;
    stall) stall ;;
    throughput) throughput ;;
    all) polling; stall; throughput ;;
    *) echo "usage: $0 [polling|stall|throughput|all]" >&2; exit 2 ;;
esac
