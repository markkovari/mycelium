wash     := "wash"
nats_url := env_var_or_default("NATS_URL", "nats://127.0.0.1:4222")

components := "gateway executor agent tool-runner memory-store conversation-store router event-logger telegram-gateway session-bridge"

default:
    @just --list

# ── Dev Lifecycle ─────────────────────────────────────────────────────────────

# Start embedded NATS + wasmCloud host
up:
    {{wash}} up --detached

# Stop wasmCloud host
down:
    {{wash}} down

# Full local dev cycle: stop → start → streams → wit-deps → build → deploy
dev: down up init-streams init-wit-deps build deploy-local
    @echo "mycelium running. HTTP on :8080, Telegram webhook on :8081"

# Deploy local manifest
deploy-local:
    {{wash}} app deploy wadm/local.yaml

# Undeploy local
undeploy:
    {{wash}} app undeploy mycelium-local || true

# Deploy dev manifest
deploy-dev:
    {{wash}} app deploy wadm/dev.yaml

# Watch wash app status
status:
    watch -n2 '{{wash}} app list'

# Tail host logs
logs:
    {{wash}} get hosts -o json | jq -r '.[0].id' | xargs -I{} {{wash}} logs {}

# ── WIT Deps ──────────────────────────────────────────────────────────────────

# Fetch external WIT deps into wit/deps/ using wkg
fetch-wit-deps:
    wkg wit fetch --dir wit

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

# Build all WASM components
build:
    #!/usr/bin/env bash
    set -euo pipefail
    for comp in {{components}}; do
        echo "==> Building $comp"
        (cd "components/$comp" && {{wash}} build)
    done
    echo "All components built."

# Build a single component: just build-one gateway
build-one comp:
    cd components/{{comp}} && {{wash}} build

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
    {{wash}} app list

hosts:
    {{wash}} get hosts

inventory:
    {{wash}} get inventory
