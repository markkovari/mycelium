#!/usr/bin/env bash
# Deploy mycelium components to a wash host v2 via NATS workload.start.
#
# v2 host imposes strict workload boundaries: WIT imports only resolve within
# ONE Workload (fully-resolved WIT World). Cross-workload calls must go through
# NATS messaging.
#
# Layout:
#   mycelium-api          = gateway + conversation-store    (HTTP-driven)
#   mycelium-telegram-in  = telegram-gateway                (HTTP webhook receiver)
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
        IFS=':' read -ra parts <<< "$spec"
        local ns="${parts[0]}"
        local pkg="${parts[1]}"
        local ifaces="${parts[2]}"
        local cfg="${parts[3]:-}"

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
            IFS=',' read -ra carr <<< "$cfg"
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
        out+="{\"image\":\"$image\",\"name\":\"$name\",\"image_pull_policy\":\"$PULL_POLICY\",\"pool_size\":${pool_size},\"max_invocations\":0}"
    done
    out+="]"
    echo "$out"
}

deploy_workload() {
    local workload_id="$1"
    local name="$2"
    local components_json="$3"
    local ifaces_json="$4"
    local payload
    payload=$(cat <<EOF
{
  "workload_id": "${workload_id}",
  "workload": {
    "namespace": "mycelium",
    "name": "${name}",
    "annotations": {"managed-by": "mycelium-deploy-v2"},
    "service": null,
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
    local reply
    reply=$(nats --server "$NATS_URL" req "runtime.host.${HOST_ID}.workload.start" "$payload" --timeout 60s 2>&1 \
        | sed -n '/{/,$p')
    echo "    $(echo "$reply" | grep -oE 'WORKLOAD_STATE_[A-Z_]+|"message":"[^"]+"' | tr '\n' ' ')"
}

# Per-workload pool_size (env-overridable).
POOL_SIZE_API="${API_POOL_SIZE:-4}"
POOL_SIZE_AGENT="${AGENT_POOL_SIZE:-8}"
POOL_SIZE_TOOLS="${TOOLS_POOL_SIZE:-4}"
POOL_SIZE_DEFAULT="${POOL_SIZE_DEFAULT:-1}"

# WORKLOADS = (workload_id;name;components;ifaces;pool_size)
# components: comma-separated component names (each pulled from $OCI_REGISTRY/<name>:$IMAGE_TAG)
# ifaces: pipe-separated specs (ns:pkg:iface1,iface2[:cfg_k=v,cfg_k=v])
WORKLOADS=(
    "mycelium-api;api;gateway,conversation-store,agent-registry;wasi:http:incoming-handler:host=localhost|wasi:keyvalue:store|wasi:logging:logging;${POOL_SIZE_API}"
    "mycelium-telegram-in;telegram-in;telegram-gateway,agent-registry;wasi:http:incoming-handler:host=telegram.localhost|wasi:config:store|wasi:keyvalue:store|wasi:logging:logging;${POOL_SIZE_DEFAULT}"
    "mycelium-telegram-out;telegram-out;telegram-out;wasi:config:store|wasi:logging:logging|wasmcloud:messaging:consumer|wasi:http:outgoing-handler;${POOL_SIZE_DEFAULT}"
    "mycelium-pairing;pairing;session-bridge;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer;${POOL_SIZE_DEFAULT}"
    "mycelium-executor;executor;executor;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer;${POOL_SIZE_DEFAULT}"
    "mycelium-agent;agent;agent;wasi:keyvalue:store|wasi:config:store|wasi:logging:logging|wasmcloud:messaging:consumer|wasi:http:outgoing-handler;${POOL_SIZE_AGENT}"
    "mycelium-tools;tools;tool-runner;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer;${POOL_SIZE_TOOLS}"
    "mycelium-memory;memory;memory-store;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer;${POOL_SIZE_DEFAULT}"
    "mycelium-router;router;router;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer;${POOL_SIZE_DEFAULT}"
    "mycelium-events;events;event-logger;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer;${POOL_SIZE_DEFAULT}"
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

            IFS=',' read -ra comp_arr <<< "$local_comps"
            comps_json=$(build_component_array "$local_pool" "${comp_arr[@]}")
            IFS='|' read -ra iface_arr <<< "$local_ifaces"
            ifaces_json=$(build_iface_array "${iface_arr[@]}")

            deploy_workload "$local_wid" "$local_name" "$comps_json" "$ifaces_json"
        done
        ;;
    undeploy)
        for spec in "${WORKLOADS[@]}"; do
            wid="${spec%%;*}"
            nats --server "$NATS_URL" req "runtime.host.${HOST_ID}.workload.stop" "{\"workload_id\":\"${wid}\"}" --timeout 10s >/dev/null 2>&1 || true
            echo "stopped $wid"
        done
        # Also stop any legacy single-component workloads
        for name in gateway executor agent tool-runner memory-store conversation-store router event-logger telegram-gateway session-bridge; do
            nats --server "$NATS_URL" req "runtime.host.${HOST_ID}.workload.stop" "{\"workload_id\":\"mycelium-${name}\"}" --timeout 5s >/dev/null 2>&1 || true
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
