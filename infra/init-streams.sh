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
create_stream MYCELIUM_TASKS          "mycelium.task.>"         file   168h
create_stream MYCELIUM_TOOL_CALLS     "mycelium.tool.>"         memory 1h
create_stream MYCELIUM_CONVERSATIONS  "mycelium.conversation.>" file   720h
create_stream MYCELIUM_EVENTS         "mycelium.event.>"        file   2160h
create_stream MYCELIUM_CHANNELS       "mycelium.channel.>"      file   24h
create_stream MYCELIUM_PAIRING        "mycelium.pair.>"         memory 10m

echo ""
echo "==> Creating KV buckets"
create_kv mycelium-task-state
create_kv mycelium-memory
create_kv mycelium-conversations
create_kv mycelium-messages        50
create_kv mycelium-agent-config
create_kv mycelium-router-rules
create_kv mycelium-channel-sessions
create_kv mycelium-events-journal  1
create_kv mycelium-telegram-poller-state
create_kv mycelium-channel-pending

echo ""
echo "Done. Run \`wash app deploy wadm/local.yaml\` to start components."
