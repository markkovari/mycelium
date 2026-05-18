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
MYCELIUM_REF="${MYCELIUM_REF:-main}"     # git ref for fetching unit files + helper scripts
MYCELIUM_PREFIX="${MYCELIUM_PREFIX:-/usr/local}"
MYCELIUM_DATA_DIR="${MYCELIUM_DATA_DIR:-/var/lib/mycelium}"
MYCELIUM_CONF_DIR="${MYCELIUM_CONF_DIR:-/etc/mycelium}"
GHCR_OWNER="${GHCR_OWNER:-markkovari}"
GHCR_REGISTRY="${GHCR_REGISTRY:-ghcr.io}"
SKIP_SYSTEMD="${SKIP_SYSTEMD:-0}"
SKIP_PULL="${SKIP_PULL:-0}"
SKIP_DEPLOY="${SKIP_DEPLOY:-0}"

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
    "$MYCELIUM_DATA_DIR/wash/config" \
    "$MYCELIUM_DATA_DIR/wash/data" \
    "$MYCELIUM_DATA_DIR/wash/cache" \
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
fi
# Always ensure $MYCELIUM_PREFIX/bin/wash exists as a regular file so systemd
# (with ProtectHome=true) can exec it; the upstream installer drops it in
# $HOME/.wash/bin which the unit's namespace cannot see.
wash_src=""
for cand in "$MYCELIUM_PREFIX/bin/wash" "$HOME/.wash/bin/wash" "$HOME/.wasmcloud/bin/wash" "$(command -v wash 2>/dev/null || true)"; do
    [ -n "$cand" ] && [ -x "$cand" ] && [ ! -L "$cand" ] && { wash_src="$cand"; break; }
done
# Fallback: accept a symlink only if the target is itself a regular file outside $HOME.
if [ -z "$wash_src" ]; then
    for cand in "$HOME/.wash/bin/wash" "$HOME/.wasmcloud/bin/wash"; do
        [ -x "$cand" ] && { wash_src="$cand"; break; }
    done
fi
[ -z "$wash_src" ] && die "wash binary not found after install"
if [ "$wash_src" != "$MYCELIUM_PREFIX/bin/wash" ]; then
    log "Copying wash from $wash_src → $MYCELIUM_PREFIX/bin/wash"
    $SUDO install -m 0755 "$wash_src" "$MYCELIUM_PREFIX/bin/wash"
fi
export PATH="$MYCELIUM_PREFIX/bin:$PATH"
log "wash: $(command -v wash) ($(wash --version 2>/dev/null | head -1))"

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
# Mycelium wash host environment (autogenerated by install.sh).
# Only shell-safe variable names are valid here (systemd EnvironmentFile).
AGENT_POOL_SIZE=$agent_pool
API_POOL_SIZE=$agent_pool
TOOLS_POOL_SIZE=$tools_pool
EOF
$SUDO chmod 600 "$MYCELIUM_CONF_DIR/host.env"

# Tokens / endpoints live in a separate secrets file consumed by deploy-v2.sh,
# NOT by systemd. Use `mycelium init` (or `mycelium provider`/`mycelium token`)
# to populate this — never hand-edit unless you know the schema.
if [ ! -f "$MYCELIUM_CONF_DIR/secrets.env" ]; then
    $SUDO tee "$MYCELIUM_CONF_DIR/secrets.env" >/dev/null <<'EOF'
# LLM_ENDPOINT=
# LLM_MODEL=
# LLM_API_KEY=
# TELEGRAM_BOT_TOKEN=
EOF
    $SUDO chmod 600 "$MYCELIUM_CONF_DIR/secrets.env"
fi

# Helper to fetch repo files at the pinned ref. Uses codeload.github.com which
# is cache-busted by the ref segment, unlike raw.githubusercontent.com.
REPO_RAW="https://raw.githubusercontent.com/${GHCR_OWNER}/mycelium/${MYCELIUM_REF}"

# ── systemd units ────────────────────────────────────────────────────────────
if [ "$SKIP_SYSTEMD" != "1" ] && command -v systemctl >/dev/null 2>&1; then
    log "Installing systemd units (ref=${MYCELIUM_REF})"
    for unit in mycelium-nats.service mycelium-host.service; do
        curl -fsSL "${REPO_RAW}/infra/systemd/${unit}" | $SUDO tee "/etc/systemd/system/${unit}" >/dev/null
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
    tmp_init=$(mktemp)
    curl -fsSL "${REPO_RAW}/infra/init-streams.sh" -o "$tmp_init"
    NATS_URL="nats://127.0.0.1:4222" bash "$tmp_init" || warn "init-streams.sh failed; run manually after fixing"
    rm -f "$tmp_init"
else
    warn "nats CLI not installed; skip stream init."
fi

# ── Install deploy-v2.sh persistently for the CLI to reuse ───────────────────
log "Installing deploy-v2.sh to /usr/local/share/mycelium/"
$SUDO mkdir -p /usr/local/share/mycelium
tmp_dep=$(mktemp)
curl -fsSL "${REPO_RAW}/infra/deploy-v2.sh" -o "$tmp_dep"
$SUDO install -m 0755 "$tmp_dep" /usr/local/share/mycelium/deploy-v2.sh
rm -f "$tmp_dep"

# ── Deploy workloads to the running host ─────────────────────────────────────
if [ "$SKIP_DEPLOY" != "1" ] && command -v nats >/dev/null 2>&1; then
    log "Deploying workloads (waiting up to 30s for host heartbeat)"
    OCI_REGISTRY="${GHCR_REGISTRY}/${GHCR_OWNER}/mycelium" \
    IMAGE_TAG="${MYCELIUM_VERSION}" \
    NATS_URL="nats://127.0.0.1:4222" \
    MYCELIUM_SECRETS_FILE="$MYCELIUM_CONF_DIR/secrets.env" \
    bash /usr/local/share/mycelium/deploy-v2.sh || warn "deploy-v2.sh failed; rerun manually with: mycelium redeploy"
else
    log "Skipping deploy (SKIP_DEPLOY=1 or nats CLI missing)"
fi

# ── Install mycelium CLI ─────────────────────────────────────────────────────
log "Installing mycelium CLI to $MYCELIUM_PREFIX/bin/mycelium"
tmp_cli=$(mktemp)
curl -fsSL "${REPO_RAW}/infra/mycelium-cli.sh" -o "$tmp_cli"
$SUDO install -m 0755 "$tmp_cli" "$MYCELIUM_PREFIX/bin/mycelium"
rm -f "$tmp_cli"

# ── Done ─────────────────────────────────────────────────────────────────────
cat <<'EOF'

Mycelium installed.

Quick start:
  mycelium provider gemini AIza...           # or: openai, anthropic, ollama
  mycelium token telegram 123:ABC            # (optional) wire up bot
  mycelium agent create alice --prompt='Be terse.'
  mycelium chat alice 'hello'
  mycelium status
  mycelium logs host

Files:
  /etc/mycelium/host.env       (tokens; chmod 600)
  /var/lib/mycelium            (wash state + NATS data)
  /etc/systemd/system/mycelium-{nats,host}.service
EOF
