#!/usr/bin/env bash
# Deploy mycelium components to a wash host v2 via NATS workload.start.
#
# v2 host imposes strict workload boundaries: WIT imports only resolve within
# ONE Workload (fully-resolved WIT World). Cross-workload calls must go through
# NATS messaging.
#
# Layout:
#   mycelium-api          = gateway + conversation-store    (HTTP-driven)
#   mycelium-telegram-poll= telegram-poller (Service)        (long-poll, no public URL needed)
#   mycelium-telegram-out = telegram-out                    (NATS → Telegram API)
#   mycelium-pairing      = session-bridge                  (NATS RPC for cli pairing)
#   mycelium-executor     = executor                        (NATS task.submit)
#   mycelium-agent        = agent                           (NATS step + LLM HTTP)
#   mycelium-tools        = tool-runner                     (NATS tool.call)
#   mycelium-memory       = memory-store                    (NATS mycelium.memory.>)
#   mycelium-router       = router                          (NATS event fan-out)
#   mycelium-events       = event-logger                    (NATS event.>)
#
# Env:
#   NATS_URL      — default nats://127.0.0.1:4222
#   OCI_REGISTRY  — default localhost:5001/mycelium
#   IMAGE_TAG     — default dev
#   HOST_ID       — auto-discovered via runtime.operator.heartbeat.> if unset
set -euo pipefail

NATS_URL="${NATS_URL:-nats://127.0.0.1:4222}"
OCI_REGISTRY="${OCI_REGISTRY:-localhost:5001/mycelium}"
IMAGE_TAG="${IMAGE_TAG:-dev}"
PULL_POLICY="${PULL_POLICY:-IMAGE_PULL_POLICY_IF_NOT_PRESENT}"

if [ -z "${HOST_ID:-}" ]; then
    HOST_ID=$(nats --server "$NATS_URL" sub 'runtime.operator.heartbeat.>' --count 1 --timeout 20s 2>/dev/null \
        | grep -oE '"id":"[^"]+"' | head -1 | cut -d'"' -f4)
fi
if [ -z "$HOST_ID" ]; then
    echo "ERROR: no wash host heartbeat seen on $NATS_URL" >&2
    exit 1
fi
echo "Target host: $HOST_ID"
echo "Registry:    $OCI_REGISTRY"
echo "Tag:         $IMAGE_TAG"

build_iface_array() {
    local out="["
    local first=1
    for spec in "$@"; do
        # Use 4-field read so cfg values can contain `:` (e.g. URLs).
        local ns pkg ifaces cfg
        IFS=':' read -r ns pkg ifaces cfg <<< "$spec"
        cfg="${cfg:-}"

        local iface_json="["
        local f=1
        IFS=',' read -ra arr <<< "$ifaces"
        for i in "${arr[@]}"; do
            if [ $f -eq 1 ]; then iface_json+="\"$i\""; f=0
            else iface_json+=",\"$i\""; fi
        done
        iface_json+="]"

        local cfg_json="{}"
        if [ -n "$cfg" ]; then
            cfg_json="{"
            local cf=1
            # cfg pairs use `+` as separator so values can contain commas
            # (e.g. messaging subscription lists).
            IFS='+' read -ra carr <<< "$cfg"
            for kv in "${carr[@]}"; do
                local k="${kv%%=*}"
                local v="${kv#*=}"
                if [ $cf -eq 1 ]; then cf=0; else cfg_json+=","; fi
                cfg_json+="\"$k\":\"$v\""
            done
            cfg_json+="}"
        fi

        if [ $first -eq 1 ]; then first=0
        else out+=","; fi
        out+="{\"namespace\":\"$ns\",\"package\":\"$pkg\",\"interfaces\":$iface_json,\"config\":$cfg_json}"
    done
    out+="]"
    echo "$out"
}

build_component_array() {
    local pool_size="$1"
    shift
    local out="["
    local first=1
    for name in "$@"; do
        local image="${OCI_REGISTRY}/${name}:${IMAGE_TAG}"
        if [ $first -eq 1 ]; then first=0
        else out+=","; fi
        out+="{\"image\":\"$image\",\"name\":\"$name\",\"image_pull_policy\":\"$PULL_POLICY\",\"pool_size\":${pool_size},\"max_invocations\":${pool_size}}"
    done
    out+="]"
    echo "$out"
}

deploy_workload() {
    local workload_id="$1"
    local name="$2"
    local components_json="$3"
    local ifaces_json="$4"
    local service_name="${5:-}"

    local service_json="null"
    if [ -n "$service_name" ]; then
        local image="${OCI_REGISTRY}/${service_name}:${IMAGE_TAG}"
        service_json="{\"image\":\"$image\",\"image_pull_policy\":\"$PULL_POLICY\",\"max_restarts\":3}"
    fi

    local payload
    payload=$(cat <<EOF
{
  "workload_id": "${workload_id}",
  "workload": {
    "namespace": "mycelium",
    "name": "${name}",
    "annotations": {"managed-by": "mycelium-deploy-v2"},
    "service": ${service_json},
    "wit_world": {
      "components": ${components_json},
      "host_interfaces": ${ifaces_json}
    },
    "volumes": []
  }
}
EOF
)
    echo "==> ${workload_id}"
    if [ "${MYCELIUM_DEPLOY_DEBUG:-0}" = "1" ]; then
        echo "$payload" >&2
    fi
    local reply
    reply=$(nats --server "$NATS_URL" req "runtime.host.${HOST_ID}.workload.start" "$payload" --timeout 60s 2>&1 \
        | sed -n '/{/,$p')
    echo "    $(echo "$reply" | grep -oE 'WORKLOAD_STATE_[A-Z_]+|"message":"[^"]+"' | tr '\n' ' ')"
}

# Optional secrets file populated by `mycelium provider` / `mycelium token`.
# Keys use shell-safe names; this script maps them to dotted wasi:config keys.
SECRETS_FILE="${MYCELIUM_SECRETS_FILE:-/etc/mycelium/secrets.env}"
if [ -f "$SECRETS_FILE" ]; then
    # shellcheck disable=SC1090
    set -a; . "$SECRETS_FILE"; set +a
fi

# Build per-workload cfg fragments out of the loaded secrets. Missing values
# are simply omitted, which keeps the workload definitions stable.
build_cfg() {
    local out=""
    while [ $# -ge 2 ]; do
        local cfg_key="$1"; shift
        local env_val="$1"; shift
        if [ -n "${env_val}" ]; then
            [ -n "$out" ] && out+="+"
            out+="${cfg_key}=${env_val}"
        fi
    done
    printf '%s' "$out"
}
AGENT_CFG="$(build_cfg \
    llm.endpoint "${LLM_ENDPOINT:-}" \
    llm.model "${LLM_MODEL:-}" \
    llm.api_key "${LLM_API_KEY:-}" \
    llm.rpm "${LLM_RPM:-}" \
    llm.rpd "${LLM_RPD:-}" \
)"
TELEGRAM_CFG="$(build_cfg telegram.bot_token "${TELEGRAM_BOT_TOKEN:-}")"
# channel-router falls back to list_agents when default.agent_id is "auto" or
# unset, but the messaging plugin only binds wasi:config when at least one key
# is configured — so always send a sentinel.
CHANNEL_CFG="$(build_cfg \
    default.agent_id "${DEFAULT_AGENT_ID:-auto}" \
    telegram.bot_token "${TELEGRAM_BOT_TOKEN:-}" \
    llm.rpm "${LLM_RPM:-}" \
    llm.rpd "${LLM_RPD:-}" \
)"

# Per-workload pool_size. Doubles as the cap on concurrent handler invocations
# (max_invocations = pool_size). Keep at 1 to serialize per workload — wash 2.1.0
# fans NATS messages to every available instance in parallel, which creates
# exponential tick storms and races every shared KV without atomics. Single-user
# setup tolerates serial.
POOL_SIZE_API="${API_POOL_SIZE:-1}"
POOL_SIZE_AGENT="${AGENT_POOL_SIZE:-1}"
POOL_SIZE_TOOLS="${TOOLS_POOL_SIZE:-1}"
POOL_SIZE_DEFAULT="${POOL_SIZE_DEFAULT:-1}"

# WORKLOADS = (workload_id;name;components;ifaces;pool_size)
# components: comma-separated component names (each pulled from $OCI_REGISTRY/<name>:$IMAGE_TAG)
# ifaces: pipe-separated specs (ns:pkg:iface1,iface2[:cfg_k=v,cfg_k=v])
WORKLOADS=(
    "mycelium-api;api;gateway,conversation-store,agent-registry;wasi:http:incoming-handler:host=localhost|wasi:keyvalue:store|wasi:logging:logging;${POOL_SIZE_API}"
    "mycelium-telegram-poll;telegram-poll;telegram-poller;wasi:config:store${TELEGRAM_CFG:+:}${TELEGRAM_CFG}|wasi:keyvalue:store|wasi:logging:logging|wasi:http:outgoing-handler|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.telegram.poll.tick;${POOL_SIZE_DEFAULT}"
    "mycelium-channel-router;channel-router;channel-router,agent-registry,conversation-store;wasi:keyvalue:store|wasi:config:store${CHANNEL_CFG:+:}${CHANNEL_CFG}|wasi:logging:logging|wasi:http:outgoing-handler|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.channel.telegram.raw;${POOL_SIZE_DEFAULT}"
    "mycelium-telegram-out;telegram-out;telegram-out;wasi:config:store${TELEGRAM_CFG:+:}${TELEGRAM_CFG}|wasi:logging:logging|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.channel.telegram.out.>;${POOL_SIZE_DEFAULT}"
    "mycelium-pairing;pairing;session-bridge;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.pair.>;${POOL_SIZE_DEFAULT}"
    "mycelium-executor;executor;executor,conversation-store;wasi:keyvalue:store|wasi:config:store${CHANNEL_CFG:+:}${CHANNEL_CFG}|wasi:logging:logging|wasi:http:outgoing-handler|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.task.submit,mycelium.step.result,mycelium.step.tool-calls,mycelium.tool.result;${POOL_SIZE_DEFAULT}"
    "mycelium-agent;agent;agent,conversation-store;wasi:keyvalue:store|wasi:config:store${AGENT_CFG:+:}${AGENT_CFG}|wasi:logging:logging|wasi:http:outgoing-handler|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.task.step.agent;${POOL_SIZE_AGENT}"
    "mycelium-tools;tools;tool-runner;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.tool.call;${POOL_SIZE_TOOLS}"
    "mycelium-memory;memory;memory-store;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.memory.>;${POOL_SIZE_DEFAULT}"
    "mycelium-router;router;router;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.event.>;${POOL_SIZE_DEFAULT}"
    "mycelium-events;events;event-logger;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.event.>;${POOL_SIZE_DEFAULT}"
    "mycelium-tool-time;tool-time;tool-time;wasi:logging:logging|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.tool.call.time;${POOL_SIZE_DEFAULT}"
    "mycelium-tool-calc;tool-calc;tool-calc;wasi:logging:logging|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.tool.call.calc;${POOL_SIZE_DEFAULT}"
    "mycelium-tool-web-fetch;tool-web-fetch;tool-web-fetch;wasi:logging:logging|wasi:http:outgoing-handler|wasmcloud:messaging:consumer,handler,types:subscriptions=mycelium.tool.call.web_fetch;${POOL_SIZE_DEFAULT}"
)

case "${1:-deploy}" in
    deploy)
        for spec in "${WORKLOADS[@]}"; do
            IFS=';' read -ra parts <<< "$spec"
            local_wid="${parts[0]}"
            local_name="${parts[1]}"
            local_comps="${parts[2]}"
            local_ifaces="${parts[3]}"
            local_pool="${parts[4]:-1}"
            local_extra="${parts[5]:-}"

            local_service=""
            if [[ "$local_extra" == SERVICE=* ]]; then
                local_service="${local_extra#SERVICE=}"
            fi

            IFS=',' read -ra comp_arr <<< "$local_comps"
            comps_json=$(build_component_array "$local_pool" "${comp_arr[@]}")
            IFS='|' read -ra iface_arr <<< "$local_ifaces"
            ifaces_json=$(build_iface_array "${iface_arr[@]}")

            deploy_workload "$local_wid" "$local_name" "$comps_json" "$ifaces_json" "$local_service"
        done
        ;;
    undeploy)
        for spec in "${WORKLOADS[@]}"; do
            wid="${spec%%;*}"
            nats --server "$NATS_URL" req "runtime.host.${HOST_ID}.workload.stop" "{\"workload_id\":\"${wid}\"}" --timeout 10s >/dev/null 2>&1 || true
            echo "stopped $wid"
        done
        # Also stop legacy workload ids (renamed or removed)
        for legacy in mycelium-gateway mycelium-conversation-store mycelium-telegram-in mycelium-telegram-gateway; do
            nats --server "$NATS_URL" req "runtime.host.${HOST_ID}.workload.stop" "{\"workload_id\":\"${legacy}\"}" --timeout 5s >/dev/null 2>&1 || true
        done
        ;;
    status)
        for spec in "${WORKLOADS[@]}"; do
            wid="${spec%%;*}"
            state=$(nats --server "$NATS_URL" req "runtime.host.${HOST_ID}.workload.status" "{\"workload_id\":\"${wid}\"}" --timeout 5s 2>&1 \
                | grep -oE 'WORKLOAD_STATE_[A-Z_]+' | head -1)
            printf "  %-22s %s\n" "$wid" "$state"
        done
        ;;
    *)
        echo "usage: $0 [deploy|undeploy|status]" >&2
        exit 2
        ;;
esac
