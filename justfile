wash         := "wash"
nats_url     := env_var_or_default("NATS_URL", "nats://127.0.0.1:4222")
nats_store   := env_var_or_default("NATS_STORE_DIR", env_var_or_default("HOME", "/root") + "/.local/share/mycelium-nats")

# v2 (default path)
oci_registry := env_var_or_default("OCI_REGISTRY", "localhost:5001/mycelium")
image_tag    := env_var_or_default("IMAGE_TAG", "dev")
http_addr    := env_var_or_default("HTTP_ADDR", "0.0.0.0:8080")

# Only the sandboxed skill components remain as wasm. Pipeline is native.
components := "tool-time tool-calc tool-web-fetch"
native_bins := "mycelium-core mycelium-tool-runner"

default:
    @just --list

# ── Dev Lifecycle (v2) ────────────────────────────────────────────────────────

# Start NATS (JetStream) + local OCI registry + wash host v2
up:
    #!/usr/bin/env bash
    set -euo pipefail
    # Auto-tune AGENT_POOL_SIZE / API_POOL_SIZE / TOOLS_POOL_SIZE if not set.
    if [ -z "${AGENT_POOL_SIZE:-}${API_POOL_SIZE:-}${TOOLS_POOL_SIZE:-}" ]; then
        eval "$(bash infra/detect-resources.sh)"
        echo "Auto-tuned pool sizes: agent=${AGENT_POOL_SIZE} api=${API_POOL_SIZE} tools=${TOOLS_POOL_SIZE}"
    fi
    echo "==> NATS (JetStream :4222, store {{nats_store}})"
    mkdir -p "{{nats_store}}"
    nats-server -js -sd "{{nats_store}}" -p 4222 &>/tmp/mycelium-nats.log &
    echo $! > /tmp/mycelium-nats.pid
    sleep 1

    echo "==> Local OCI registry (:5001)"
    if ! docker ps --format '{{{{.Names}}}}' | grep -q '^oci-registry$'; then
        docker rm -f oci-registry 2>/dev/null || true
        docker run -d --name oci-registry -p 5001:5000 registry:2 >/dev/null
        sleep 2
    fi

    echo "==> wash host v2 (HTTP {{http_addr}})"
    wash host \
        --scheduler-nats-url {{nats_url}} \
        --data-nats-url {{nats_url}} \
        --http-addr {{http_addr}} \
        --allow-insecure-registries \
        --non-interactive &>/tmp/mycelium-v2host.log &
    echo $! > /tmp/mycelium-v2host.pid
    sleep 4

    host_id=$(nats --server {{nats_url}} sub 'runtime.operator.heartbeat.>' --count 1 --timeout 10s 2>/dev/null \
        | grep -oE '"id":"[^"]+"' | head -1 | cut -d'"' -f4)
    if [ -z "$host_id" ]; then
        echo "ERROR: no host heartbeat" >&2
        exit 1
    fi
    echo "$host_id" > /tmp/mycelium-v2-host-id
    echo "wash host v2 up. host_id=$host_id"

# Stop wash host, registry, NATS
down:
    #!/usr/bin/env bash
    for svc in v2host nats; do
        pid_file="/tmp/mycelium-$svc.pid"
        if [ -f "$pid_file" ]; then
            kill "$(cat $pid_file)" 2>/dev/null && echo "stopped $svc" || true
            rm -f "$pid_file"
        fi
    done
    docker rm -f oci-registry 2>/dev/null && echo "stopped oci-registry" || true
    rm -f /tmp/mycelium-v2-host-id

# Full cycle: down → up → streams → wit-deps → build → push → deploy
dev: down up init-streams init-wit-deps build push-oci deploy
    @echo "mycelium running on v2. HTTP on {{http_addr}}"

# Push all components to OCI registry
push-oci:
    #!/usr/bin/env bash
    set -euo pipefail
    for comp in {{components}}; do
        case "$comp" in
            tool-runner) file=tool_runner ;;
            memory-store) file=memory_store ;;
            conversation-store) file=conversation_store ;;
            event-logger) file=event_logger ;;
            telegram-poller) file=telegram_poller ;;
            telegram-out) file=telegram_out ;;
            channel-router) file=channel_router ;;
            session-bridge) file=session_bridge ;;
            agent-registry) file=agent_registry ;;
            task-publisher) file=task_publisher ;;
            tool-time) file=tool_time ;;
            tool-calc) file=tool_calc ;;
            tool-web-fetch) file=tool_web_fetch ;;
            *) file="$comp" ;;
        esac
        insecure=""
        case "{{oci_registry}}" in
            localhost:*|127.0.0.1:*) insecure="--insecure" ;;
        esac
        wash oci push $insecure "{{oci_registry}}/${comp}:{{image_tag}}" \
            "target/wasm32-wasip2/release/${file}.wasm"
        echo "pushed ${comp}"
    done

# Deploy via NATS workload.start
deploy:
    #!/usr/bin/env bash
    set -euo pipefail
    HOST_ID=$(cat /tmp/mycelium-v2-host-id 2>/dev/null) \
    NATS_URL={{nats_url}} \
    OCI_REGISTRY={{oci_registry}} \
    IMAGE_TAG={{image_tag}} \
        bash infra/deploy-v2.sh deploy

# Stop all workloads
undeploy:
    #!/usr/bin/env bash
    HOST_ID=$(cat /tmp/mycelium-v2-host-id 2>/dev/null) \
    NATS_URL={{nats_url}} \
        bash infra/deploy-v2.sh undeploy

# Show workload status
status:
    #!/usr/bin/env bash
    HOST_ID=$(cat /tmp/mycelium-v2-host-id 2>/dev/null) \
    NATS_URL={{nats_url}} \
        bash infra/deploy-v2.sh status

# Tail host logs
logs:
    tail -f /tmp/mycelium-v2host.log

# ── WIT Deps ──────────────────────────────────────────────────────────────────

fetch-wit-deps:
    #!/usr/bin/env bash
    set -euo pipefail
    tmpdir=$(mktemp -d)
    trap "rm -rf $tmpdir" EXIT
    mkdir -p "$tmpdir/deps"
    cp wit/deps.toml "$tmpdir/deps.toml"
    cp infra/fetch-world.wit "$tmpdir/world.wit"
    wkg wit fetch -d "$tmpdir"
    mkdir -p wit/deps
    cp -r "$tmpdir/deps/". wit/deps/
    # Overlay hand-maintained packages (wkg can't fetch these correctly):
    # - wasmcloud:messaging  (registry version has wrong field order; v2 host
    #   plugin's canonical layout differs from what's published)
    # - wasi:cli/run         (wkg fetch of wasi:cli omits the `run` interface)
    if [ -d wit/local-deps ]; then
        cp -R wit/local-deps/. wit/deps/
        echo "Overlaid wit/local-deps/ onto wit/deps/"
    fi
    echo "Fetched external WIT deps into wit/deps/"

init-wit-deps: fetch-wit-deps
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Seeding component wit/deps/ from workspace wit/"
    for comp in {{components}}; do
        dest="components/$comp/wit/deps"
        mkdir -p "$dest"
        rm -f "$dest/deps.toml" "$dest/world.wit"
        if [ -d "wit/deps" ]; then
            cp -r wit/deps/. "$dest/"
        fi
        for pkg in types agent conversation tool memory executor router channel pairing cron batch task; do
            mkdir -p "$dest/mycelium-${pkg}-0.1.0"
            cp "wit/${pkg}.wit" "$dest/mycelium-${pkg}-0.1.0/"
        done
        echo "  seeded components/$comp/wit/deps/"
    done

# ── NATS Streams ──────────────────────────────────────────────────────────────

init-streams:
    NATS_URL={{nats_url}} bash infra/init-streams.sh

# ── Build ─────────────────────────────────────────────────────────────────────

build:
    #!/usr/bin/env bash
    set -euo pipefail
    for comp in {{components}}; do
        echo "==> Building $comp"
        (cd "components/$comp" && cargo component build --target wasm32-wasip2 --release)
    done
    echo "All components built."

build-one comp:
    cd components/{{comp}} && cargo component build --target wasm32-wasip2 --release

# ── CLI ───────────────────────────────────────────────────────────────────────

run-cli *args:
    cargo run -p mycelium-cli -- --nats-url {{nats_url}} {{args}}

run-cli-unpaired:
    cargo run -p mycelium-cli -- --nats-url {{nats_url}} --skip-pairing

# ── Tests ─────────────────────────────────────────────────────────────────────

test:
    cargo test -p mycelium-types -p mycelium-agent-core -p mycelium-tool-core

test-lattice:
    cargo test -p mycelium-tests -- --test-threads=1 --nocapture

# ── Quality ───────────────────────────────────────────────────────────────────

check:
    cargo fmt --all -- --check
    cargo clippy -p mycelium-types -p mycelium-agent-core -p mycelium-tool-core -p mycelium-cli -- -D warnings

fmt:
    cargo fmt --all

# ── Inventory ─────────────────────────────────────────────────────────────────

hosts:
    nats --server {{nats_url}} sub 'runtime.operator.heartbeat.>' --count=1 2>/dev/null
