#!/usr/bin/env bash
# Idempotent NATS JetStream stream + KV bucket setup.
# Run: just init-streams
set -euo pipefail

NATS_URL="${NATS_URL:-nats://127.0.0.1:4222}"
nats=("nats" "--server" "$NATS_URL")

create_stream() {
    local name="$1" subjects="$2" storage="$3" max_age="$4"
    if "${nats[@]}" stream info "$name" &>/dev/null; then
        echo "  stream $name already exists — skipping"
        return
    fi
    "${nats[@]}" stream add "$name" \
        --subjects="$subjects" \
        --storage="$storage" \
        --retention=limits \
        --max-age="$max_age" \
        --replicas=1 \
        --defaults \
        --no-headers-only
    echo "  created stream $name"
}

create_kv() {
    local bucket="$1" history="${2:-10}"
    if "${nats[@]}" kv info "$bucket" &>/dev/null; then
        echo "  KV bucket $bucket already exists — skipping"
        return
    fi
    "${nats[@]}" kv add "$bucket" --replicas=1 --history="$history"
    echo "  created KV bucket $bucket"
}

echo "==> Creating JetStream streams"
create_stream LC_TASKS          "lc.task.>"         file   168h
create_stream LC_TOOL_CALLS     "lc.tool.>"         memory 1h
create_stream LC_CONVERSATIONS  "lc.conversation.>" file   720h
create_stream LC_EVENTS         "lc.event.>"        file   2160h
create_stream LC_CHANNELS       "lc.channel.>"      file   24h
create_stream LC_PAIRING        "lc.pair.>"         memory 10m

echo ""
echo "==> Creating KV buckets"
create_kv lc-task-state
create_kv lc-memory
create_kv lc-conversations
create_kv lc-messages        50
create_kv lc-agent-config
create_kv lc-router-rules
create_kv lc-channel-sessions
create_kv lc-events-journal  1

echo ""
echo "Done. Run \`wash app deploy wadm/local.yaml\` to start components."
