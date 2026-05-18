#!/usr/bin/env bash
# Compute per-workload pool_size from available memory and CPU count.
# Prints `export AGENT_POOL_SIZE=…` lines so callers can `eval "$(infra/detect-resources.sh)"`.
#
# Honours pre-set env vars: anything already exported is left alone so operators
# can pin specific pools (`AGENT_POOL_SIZE=4 just up` etc.).

set -euo pipefail

# Detect total memory (MB)
if [ -r /proc/meminfo ]; then
    total_mb=$(awk '/^MemTotal/ {print int($2/1024); exit}' /proc/meminfo)
elif command -v sysctl >/dev/null 2>&1; then
    # macOS fallback (dev convenience only)
    total_mb=$(( $(sysctl -n hw.memsize 2>/dev/null || echo 0) / 1048576 ))
else
    total_mb=2048
fi

# Detect cores
if command -v nproc >/dev/null 2>&1; then
    cores=$(nproc)
elif command -v sysctl >/dev/null 2>&1; then
    cores=$(sysctl -n hw.ncpu 2>/dev/null || echo 1)
else
    cores=1
fi

# Reservations:
#   OS baseline            ~400 MB
#   Mycelium static        ~200 MB (NATS + wash host + 11 single-instance components)
#   Headroom / file cache  ~200 MB
reserved=800
budget=$(( total_mb - reserved ))
if [ "$budget" -lt 0 ]; then
    budget=0
fi

# Each extra WASM instance ~5 MB resident
per_inst=5
total_slots=$(( budget / per_inst ))

# Per-pool cap: more than cores*2 is just wasted RAM, no throughput gain
cap=$(( cores * 2 ))
[ "$cap" -lt 1 ] && cap=1

# Distribute: agent 50%, api 25%, tools 15%, leftover unused (= safety)
agent=$(( total_slots * 50 / 100 ))
api=$(( total_slots * 25 / 100 ))
tools=$(( total_slots * 15 / 100 ))

# Apply per-pool cap and floor at 1
clamp() {
    local v="$1"
    [ "$v" -gt "$cap" ] && v="$cap"
    [ "$v" -lt 1 ] && v=1
    echo "$v"
}
agent=$(clamp "$agent")
api=$(clamp "$api")
tools=$(clamp "$tools")

# Don't override anything the operator already set
[ -n "${AGENT_POOL_SIZE:-}" ] && agent="$AGENT_POOL_SIZE"
[ -n "${API_POOL_SIZE:-}" ]   && api="$API_POOL_SIZE"
[ -n "${TOOLS_POOL_SIZE:-}" ] && tools="$TOOLS_POOL_SIZE"

cat <<EOF
# auto-detected: ${total_mb} MB RAM, ${cores} cores, budget ${budget} MB, cap ${cap}/pool
export AGENT_POOL_SIZE=${agent}
export API_POOL_SIZE=${api}
export TOOLS_POOL_SIZE=${tools}
EOF
