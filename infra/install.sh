#!/usr/bin/env bash
# Mycelium installer for Linux (aarch64 / x86_64).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/markkovari/mycelium/main/infra/install.sh | bash
#
# Or with env overrides:
#   MYCELIUM_VERSION=latest \
#   MYCELIUM_PREFIX=/usr/local \
#   MYCELIUM_DATA_DIR=/var/lib/mycelium \
#   GHCR_OWNER=markkovari \
#   curl -fsSL https://… | bash

set -euo pipefail

# ── Defaults ─────────────────────────────────────────────────────────────────
MYCELIUM_VERSION="${MYCELIUM_VERSION:-dev}"
MYCELIUM_PREFIX="${MYCELIUM_PREFIX:-/usr/local}"
MYCELIUM_DATA_DIR="${MYCELIUM_DATA_DIR:-/var/lib/mycelium}"
MYCELIUM_CONF_DIR="${MYCELIUM_CONF_DIR:-/etc/mycelium}"
GHCR_OWNER="${GHCR_OWNER:-markkovari}"
GHCR_REGISTRY="${GHCR_REGISTRY:-ghcr.io}"
SKIP_SYSTEMD="${SKIP_SYSTEMD:-0}"
SKIP_PULL="${SKIP_PULL:-0}"

WASH_VERSION="${WASH_VERSION:-2.1.0}"
NATS_VERSION="${NATS_VERSION:-2.10.20}"
NATS_CLI_VERSION="${NATS_CLI_VERSION:-0.1.5}"

COMPONENTS=(
    gateway executor agent tool-runner memory-store conversation-store
    router event-logger telegram-poller telegram-out channel-router
    session-bridge agent-registry task-publisher
)

# ── Helpers ──────────────────────────────────────────────────────────────────
log() { printf '\033[1;36m[install]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[install]\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m[install]\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }

# ── Pre-flight ───────────────────────────────────────────────────────────────
log "Mycelium installer starting"

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Linux) ;;
    *) die "this installer supports Linux only; got $OS" ;;
esac
case "$ARCH" in
    aarch64|arm64) ARCH_GO=arm64 ARCH_TAR=arm64 ;;
    x86_64|amd64)  ARCH_GO=amd64 ARCH_TAR=amd64 ;;
    *) die "unsupported architecture: $ARCH" ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    SUDO="sudo"
else
    SUDO=""
fi

need curl
need tar
need awk

# ── Layout ───────────────────────────────────────────────────────────────────
$SUDO mkdir -p \
    "$MYCELIUM_PREFIX/bin" \
    "$MYCELIUM_DATA_DIR/nats" \
    "$MYCELIUM_DATA_DIR/oci-cache" \
    "$MYCELIUM_CONF_DIR" \
    /var/log/mycelium

# ── nats-server ──────────────────────────────────────────────────────────────
if ! command -v nats-server >/dev/null 2>&1; then
    log "Installing nats-server $NATS_VERSION"
    tmp=$(mktemp -d)
    url="https://github.com/nats-io/nats-server/releases/download/v${NATS_VERSION}/nats-server-v${NATS_VERSION}-linux-${ARCH_GO}.tar.gz"
    curl -fsSL "$url" -o "$tmp/nats.tar.gz"
    tar -xzf "$tmp/nats.tar.gz" -C "$tmp"
    $SUDO install -m 0755 "$tmp"/nats-server-*/nats-server "$MYCELIUM_PREFIX/bin/nats-server"
    rm -rf "$tmp"
else
    log "nats-server already installed: $(command -v nats-server)"
fi

# ── nats CLI ─────────────────────────────────────────────────────────────────
if ! command -v nats >/dev/null 2>&1; then
    log "Installing nats CLI $NATS_CLI_VERSION"
    tmp=$(mktemp -d)
    url="https://github.com/nats-io/natscli/releases/download/v${NATS_CLI_VERSION}/nats-${NATS_CLI_VERSION}-linux-${ARCH_TAR}.zip"
    if curl -fsSL "$url" -o "$tmp/nats.zip"; then
        if command -v unzip >/dev/null 2>&1; then
            unzip -q "$tmp/nats.zip" -d "$tmp"
            $SUDO install -m 0755 "$tmp"/nats-*/nats "$MYCELIUM_PREFIX/bin/nats"
        else
            warn "  unzip not installed; skipping nats CLI install (apt install unzip and rerun)"
        fi
    else
        warn "  failed to download nats CLI; install manually from https://github.com/nats-io/natscli/releases"
    fi
    rm -rf "$tmp"
else
    log "nats CLI already installed: $(command -v nats)"
fi

# ── wash CLI ─────────────────────────────────────────────────────────────────
if ! command -v wash >/dev/null 2>&1; then
    log "Installing wash $WASH_VERSION via the official installer"
    if ! curl -fsSL https://wasmcloud.com/sh | bash; then
        die "wash install failed; try installing manually from https://github.com/wasmCloud/wash/releases"
    fi
    # The installer drops wash in ~/.wash/bin (current) or ~/.wasmcloud (older); copy
    # to $MYCELIUM_PREFIX/bin so systemd (with ProtectHome=true) can exec it.
    wash_src=""
    for cand in "$HOME/.wash/bin/wash" "$HOME/.wasmcloud/bin/wash" "/usr/local/bin/wash"; do
        if [ -x "$cand" ]; then wash_src="$cand"; break; fi
    done
    [ -z "$wash_src" ] && die "wash installed but binary not found in ~/.wash/bin or ~/.wasmcloud/bin"
    if [ "$wash_src" != "$MYCELIUM_PREFIX/bin/wash" ]; then
        $SUDO install -m 0755 "$wash_src" "$MYCELIUM_PREFIX/bin/wash"
    fi
    export PATH="$MYCELIUM_PREFIX/bin:$PATH"
else
    log "wash already installed: $(command -v wash)"
fi

# ── Verify registry reachability ─────────────────────────────────────────────
# wash host pulls components on demand into --oci-cache-dir. We only verify
# that the registry is reachable + each manifest exists at the requested tag.
if [ "$SKIP_PULL" != "1" ]; then
    log "Probing ${GHCR_REGISTRY}/${GHCR_OWNER}/mycelium/*:${MYCELIUM_VERSION}"
    ok=0; bad=""
    for c in "${COMPONENTS[@]}"; do
        scope="repository:${GHCR_OWNER}/mycelium/${c}:pull"
        tok=$(curl -fsSL "https://${GHCR_REGISTRY}/token?scope=${scope}" 2>/dev/null \
              | awk -F'"' '/token/ {for(i=1;i<=NF;i++) if($i=="token"){print $(i+2); exit}}')
        code=$(curl -sI -o /dev/null -w '%{http_code}' \
               -H "Authorization: Bearer ${tok}" \
               -H 'Accept: application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json' \
               "https://${GHCR_REGISTRY}/v2/${GHCR_OWNER}/mycelium/${c}/manifests/${MYCELIUM_VERSION}")
        if [ "$code" = "200" ]; then ok=$((ok+1)); else bad="$bad ${c}(${code})"; fi
    done
    log "  ${ok}/${#COMPONENTS[@]} manifests reachable"
    if [ -n "$bad" ]; then
        warn "  unreachable:$bad"
        warn "  host will retry on first deploy; check package visibility on ghcr.io if persistent"
    fi
else
    log "Skipping registry probe (SKIP_PULL=1)"
fi

# ── Auto-tune pool sizes ─────────────────────────────────────────────────────
log "Detecting resources for pool sizes"
total_mb=$(awk '/^MemTotal/ {print int($2/1024); exit}' /proc/meminfo)
cores=$(nproc)
reserved=800
budget=$(( total_mb - reserved ))
[ "$budget" -lt 0 ] && budget=0
per_inst=5
total_slots=$(( budget / per_inst ))
cap=$(( cores * 2 ))
[ "$cap" -lt 1 ] && cap=1
agent_pool=$(( total_slots * 50 / 100 )); [ "$agent_pool" -gt "$cap" ] && agent_pool=$cap; [ "$agent_pool" -lt 1 ] && agent_pool=1
api_pool=$(( total_slots * 25 / 100 ));   [ "$api_pool" -gt "$cap" ] && api_pool=$cap; [ "$api_pool" -lt 1 ] && api_pool=1
tools_pool=$(( total_slots * 15 / 100 )); [ "$tools_pool" -gt "$cap" ] && tools_pool=$cap; [ "$tools_pool" -lt 1 ] && tools_pool=1
log "  RAM ${total_mb}MB, cores ${cores} → agent=${agent_pool} api=${api_pool} tools=${tools_pool}"

# ── Write env file ───────────────────────────────────────────────────────────
log "Writing $MYCELIUM_CONF_DIR/host.env"
$SUDO tee "$MYCELIUM_CONF_DIR/host.env" >/dev/null <<EOF
# Mycelium wash host environment (autogenerated by install.sh)
AGENT_POOL_SIZE=$agent_pool
API_POOL_SIZE=$api_pool
TOOLS_POOL_SIZE=$tools_pool
# Optional: paste tokens you want available to components via wasi:config/store.
# telegram.bot_token=
# llm.api_key=
EOF
$SUDO chmod 600 "$MYCELIUM_CONF_DIR/host.env"

# ── systemd units ────────────────────────────────────────────────────────────
if [ "$SKIP_SYSTEMD" != "1" ] && command -v systemctl >/dev/null 2>&1; then
    log "Installing systemd units"
    BASE_URL="https://raw.githubusercontent.com/${GHCR_OWNER}/mycelium/main/infra/systemd"
    for unit in mycelium-nats.service mycelium-host.service; do
        curl -fsSL "${BASE_URL}/${unit}" | $SUDO tee "/etc/systemd/system/${unit}" >/dev/null
    done
    $SUDO systemctl daemon-reload
    $SUDO systemctl enable --now mycelium-nats.service
    $SUDO systemctl enable --now mycelium-host.service
    sleep 3
    log "Services:"
    $SUDO systemctl --no-pager --lines 0 status mycelium-nats.service mycelium-host.service || true
else
    log "Skipping systemd setup"
fi

# ── Bootstrap NATS streams + KV ──────────────────────────────────────────────
if command -v nats >/dev/null 2>&1; then
    log "Initialising NATS streams and KV buckets"
    BASE_URL="https://raw.githubusercontent.com/${GHCR_OWNER}/mycelium/main/infra"
    tmp_init=$(mktemp)
    curl -fsSL "${BASE_URL}/init-streams.sh" -o "$tmp_init"
    NATS_URL="nats://127.0.0.1:4222" bash "$tmp_init" || warn "init-streams.sh failed; run manually after fixing"
    rm -f "$tmp_init"
else
    warn "nats CLI not installed; skip stream init. Install via: go install github.com/nats-io/natscli/nats@latest"
fi

# ── Done ─────────────────────────────────────────────────────────────────────
cat <<'EOF'

Mycelium installed.

Next:
  1. Add your tokens to /etc/mycelium/host.env (telegram.bot_token, llm.api_key, …)
  2. systemctl restart mycelium-host
  3. Visit http://<this-host>:8080/health
  4. Register an agent:
       curl -X POST http://localhost:8080/agents \
         -H 'host: localhost' -H 'content-type: application/json' \
         -d '{"id":"alice","name":"Alice","system_prompt":"Be terse.","model":"gpt-4o-mini","tools":[],"max_steps":4}'

Logs:
  journalctl -u mycelium-host -f
  journalctl -u mycelium-nats -f
EOF
