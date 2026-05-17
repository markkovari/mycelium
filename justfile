wash     := "wash"
nats_url := env_var_or_default("NATS_URL", "nats://127.0.0.1:4222")
wadm_nats := env_var_or_default("WADM_NATS", "127.0.0.1:4222")
host_bin  := env_var_or_default("WASMCLOUD_HOST_BIN", "$HOME/.wash/downloads/v1.6.2/wasmcloud_host")

components := "gateway executor agent tool-runner memory-store conversation-store router event-logger telegram-gateway session-bridge"

default:
    @just --list

# ── Dev Lifecycle ─────────────────────────────────────────────────────────────

# Start NATS (JetStream), wasmCloud host, and wadm
up:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Starting NATS (JetStream on :4222)"
    nats-server -js -p 4222 &>/tmp/mycelium-nats.log &
    echo $! > /tmp/mycelium-nats.pid
    sleep 1

    echo "==> Starting wasmCloud host"
    eval "$host_bin" --nats-host 127.0.0.1 --nats-port 4222 --lattice default &>/tmp/mycelium-host.log &
    echo $! > /tmp/mycelium-host.pid
    sleep 2

    echo "==> Starting wadm"
    wadm --nats-server {{wadm_nats}} --http-admin 127.0.0.1:9999 &>/tmp/mycelium-wadm.log &
    echo $! > /tmp/mycelium-wadm.pid
    sleep 2

    echo "Services started. Logs: /tmp/mycelium-{nats,host,wadm}.log"

# Stop all services
down:
    #!/usr/bin/env bash
    for svc in wadm host nats; do
        pid_file="/tmp/mycelium-$svc.pid"
        if [ -f "$pid_file" ]; then
            pid=$(cat "$pid_file")
            kill "$pid" 2>/dev/null && echo "Stopped $svc (PID $pid)" || echo "$svc already stopped"
            rm -f "$pid_file"
        fi
    done

# Full local dev cycle: stop → start → streams → wit-deps → build → deploy
dev: down up init-streams init-wit-deps build deploy-local
    @echo "mycelium running. HTTP on :8080, Telegram webhook on :8081"

# Deploy local manifest via wadm NATS API
deploy-local:
    #!/usr/bin/env bash
    set -euo pipefail
    name=mycelium-local
    result=$(nats --server {{nats_url}} req wadm.api.default.model.put "$(cat wadm/local.yaml)" 2>&1)
    echo "$result" | grep -q '"error"' && echo "$result" | grep -v 'already exists' | grep '"error"' && exit 1 || true
    nats --server {{nats_url}} req wadm.api.default.model.deploy.$name '{"version":"v0.1.0"}'
    echo "Deployed $name"

# Undeploy local
undeploy:
    nats --server {{nats_url}} req wadm.api.default.model.undeploy.mycelium-local '{}' || true

# Deploy dev manifest
deploy-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    name=mycelium-dev
    nats --server {{nats_url}} req wadm.api.default.model.put "$(cat wadm/dev.yaml)"
    nats --server {{nats_url}} req wadm.api.default.model.deploy.$name '{"version":"v0.1.0"}'
    echo "Deployed $name"

# Watch app status
status:
    watch -n2 'nats --server {{nats_url}} req wadm.api.default.model.status.mycelium-local "" 2>/dev/null | python3 -c "import json,sys; d=json.load(sys.stdin); print(d[\"status\"][\"status\"][\"type\"])"'

# Tail host logs
logs:
    tail -f /tmp/mycelium-host.log

# ── WIT Deps ──────────────────────────────────────────────────────────────────

# Fetch external WIT deps into wit/deps/ using wkg.
# Uses a temporary single-package dir to avoid wkg's multi-package restriction.
fetch-wit-deps:
    #!/usr/bin/env bash
    set -euo pipefail
    tmpdir=$(mktemp -d)
    trap "rm -rf $tmpdir" EXIT
    mkdir -p "$tmpdir/deps"
    # Pre-populate internal packages so wkg skips registry lookup for them
    for wit_file in wit/types.wit wit/agent.wit wit/conversation.wit wit/tool.wit wit/memory.wit wit/executor.wit wit/router.wit wit/channel.wit wit/pairing.wit wit/cron.wit wit/batch.wit; do
        pkg=$(head -1 "$wit_file" | sed 's/package //' | sed 's/;//')
        name=$(echo "$pkg" | tr ':@.' '-')
        mkdir -p "$tmpdir/deps/$name"
        cp "$wit_file" "$tmpdir/deps/$name/"
    done
    cp wit/deps.toml "$tmpdir/deps.toml"
    cp components/gateway/wit/world.wit "$tmpdir/world.wit"
    wkg wit fetch -d "$tmpdir"
    mkdir -p wit/deps
    # Copy only externally-fetched deps (not our internal mycelium-* packages)
    for d in "$tmpdir/deps/"*/; do
        pkg_name=$(basename "$d")
        if [[ "$pkg_name" != mycelium-* ]]; then
            cp -r "$d" wit/deps/
        fi
    done
    echo "Fetched external WIT deps into wit/deps/"

# Seed each component's wit/deps/ from the workspace wit/ directory.
# Run once after clone, and whenever wit/*.wit or wit/deps/ changes.
init-wit-deps: fetch-wit-deps
    #!/usr/bin/env bash
    set -euo pipefail

    echo "==> Seeding component wit/deps/ from workspace wit/"

    # Internal mycelium packages to copy
    declare -A pkg_dirs=(
        ["mycelium-types-0.1.0"]="wit/types.wit"
        ["mycelium-agent-0.1.0"]="wit/agent.wit"
        ["mycelium-conversation-0.1.0"]="wit/conversation.wit"
        ["mycelium-tool-0.1.0"]="wit/tool.wit"
        ["mycelium-memory-0.1.0"]="wit/memory.wit"
        ["mycelium-executor-0.1.0"]="wit/executor.wit"
        ["mycelium-router-0.1.0"]="wit/router.wit"
        ["mycelium-channel-0.1.0"]="wit/channel.wit"
        ["mycelium-pairing-0.1.0"]="wit/pairing.wit"
        ["mycelium-cron-0.1.0"]="wit/cron.wit"
        ["mycelium-batch-0.1.0"]="wit/batch.wit"
    )

    for comp in {{components}}; do
        dest="components/$comp/wit/deps"
        mkdir -p "$dest"

        # Copy external deps
        if [ -d "wit/deps" ]; then
            cp -r wit/deps/. "$dest/"
        fi

        # Copy internal packages
        for pkg_dir in "${!pkg_dirs[@]}"; do
            src="${pkg_dirs[$pkg_dir]}"
            mkdir -p "$dest/$pkg_dir"
            cp "$src" "$dest/$pkg_dir/"
        done

        echo "  seeded components/$comp/wit/deps/"
    done

    echo "Done."

# ── NATS Streams ──────────────────────────────────────────────────────────────

# Create JetStream streams and KV buckets (idempotent)
init-streams:
    NATS_URL={{nats_url}} bash infra/init-streams.sh

# ── Build ─────────────────────────────────────────────────────────────────────

# Build all WASM components (requires: cargo install cargo-component)
build:
    #!/usr/bin/env bash
    set -euo pipefail
    for comp in {{components}}; do
        echo "==> Building $comp"
        (cd "components/$comp" && cargo component build --target wasm32-wasip2 --release)
    done
    echo "All components built."

# Build a single component: just build-one gateway
build-one comp:
    cd components/{{comp}} && cargo component build --target wasm32-wasip2

# ── CLI ───────────────────────────────────────────────────────────────────────

# Run the interactive CLI REPL
run-cli *args:
    cargo run -p mycelium-cli -- --nats-url {{nats_url}} {{args}}

# Run CLI unpaired (skip Telegram pairing)
run-cli-unpaired:
    cargo run -p mycelium-cli -- --nats-url {{nats_url}} --skip-pairing

# ── Tests ─────────────────────────────────────────────────────────────────────

# Run unit tests (non-WASM crates only)
test:
    cargo test -p mycelium-types -p mycelium-agent-core -p mycelium-tool-core

# Run lattice integration tests (requires `just up && just init-streams && just deploy-local`)
test-lattice:
    cargo test -p mycelium-tests -- --test-threads=1 --nocapture

# ── Quality ───────────────────────────────────────────────────────────────────

check:
    cargo fmt --all -- --check
    cargo clippy -p mycelium-types -p mycelium-agent-core -p mycelium-tool-core -p mycelium-cli -- -D warnings

fmt:
    cargo fmt --all

# ── Inventory ─────────────────────────────────────────────────────────────────

apps:
    nats --server {{nats_url}} req wadm.api.default.model.list '' 2>/dev/null

hosts:
    nats --server {{nats_url}} sub 'wasmbus.evt.default.host.heartbeat' --count=1 2>/dev/null
