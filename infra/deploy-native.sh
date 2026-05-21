#!/usr/bin/env bash
# Cut over a mycelium Pi from the wash-based stack to the native
# mycelium-core + mycelium-tool-runner binaries.
#
# Idempotent: rerun to upgrade. Stops mycelium-host before starting the
# native services so neither side races NATS subjects.
#
# Required env / files:
#   /etc/mycelium/secrets.env   — TELEGRAM_BOT_TOKEN, LLM_*, NATS_URL, etc.
#                                 (already populated for the wash deploy)
#
# Run:
#   curl -fsSL https://raw.githubusercontent.com/markkovari/mycelium/main/infra/deploy-native.sh \
#     | sudo MYCELIUM_REF=main bash
#
# Or from a checkout on the Pi:
#   sudo bash infra/deploy-native.sh
set -euo pipefail

# ── settings ────────────────────────────────────────────────────────────
GHCR_OWNER="${GHCR_OWNER:-markkovari}"
MYCELIUM_REPO="${MYCELIUM_REPO:-${GHCR_OWNER}/mycelium}"
MYCELIUM_REF="${MYCELIUM_REF:-main}"
PREFIX="${PREFIX:-/usr/local}"
STATE_DIR="${STATE_DIR:-/var/lib/mycelium}"
SECRETS_FILE="${SECRETS_FILE:-/etc/mycelium/secrets.env}"
USER_NAME="${USER_NAME:-mycelium}"
GROUP_NAME="${GROUP_NAME:-mycelium}"

log()  { printf '\033[1;36m[deploy-native]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[deploy-native]\033[0m %s\n' "$*" >&2; exit 1; }
warn() { printf '\033[1;33m[deploy-native]\033[0m %s\n' "$*" >&2; }

[ "$EUID" -eq 0 ] || die "run as root (sudo)"

# ── detect arch ─────────────────────────────────────────────────────────
ARCH_RUST=""
case "$(uname -m)" in
  aarch64|arm64) ARCH_RUST="aarch64-unknown-linux-gnu" ;;
  x86_64|amd64)  ARCH_RUST="x86_64-unknown-linux-gnu" ;;
  *) die "unsupported arch: $(uname -m)" ;;
esac
log "host arch: $ARCH_RUST"

# ── user + dirs ─────────────────────────────────────────────────────────
if ! id "$USER_NAME" >/dev/null 2>&1; then
  log "creating system user $USER_NAME"
  useradd --system --no-create-home --shell /usr/sbin/nologin "$USER_NAME"
fi
mkdir -p "$STATE_DIR" "$STATE_DIR/skill-cache"
chown -R "$USER_NAME:$GROUP_NAME" "$STATE_DIR"

# ── fetch binaries ──────────────────────────────────────────────────────
# Prefer the most recent `dev-*` workflow artifact for the given branch.
# Fall back to building from a checkout if `gh` is missing.
fetch_binary() {
  local bin="$1"
  local out="$2"
  # Workflow artifact name pattern: <bin>-dev-<sha7>-<target>.tar.gz
  if command -v gh >/dev/null 2>&1; then
    log "downloading $bin via gh (workflow artifact)"
    local run_id
    run_id=$(gh -R "$MYCELIUM_REPO" run list \
      --workflow=native.yaml --branch="$MYCELIUM_REF" --status=success \
      --limit 1 --json databaseId --jq '.[0].databaseId') || true
    if [ -n "$run_id" ]; then
      local tmpdir
      tmpdir=$(mktemp -d)
      gh -R "$MYCELIUM_REPO" run download "$run_id" --dir "$tmpdir" \
        --pattern "${bin}-*-${ARCH_RUST}.tar.gz" || warn "  gh download failed"
      local tar
      tar=$(find "$tmpdir" -name "${bin}-*-${ARCH_RUST}.tar.gz" | head -1)
      if [ -n "$tar" ]; then
        tar -xzf "$tar" -C "$tmpdir"
        install -m 0755 "$tmpdir/$bin" "$out"
        rm -rf "$tmpdir"
        return 0
      fi
      rm -rf "$tmpdir"
    fi
  fi
  warn "couldn't fetch prebuilt $bin; falling back to local cargo build"
  command -v cargo >/dev/null || die "cargo missing; install rustup first"
  ( cd /tmp && \
    [ -d mycelium-src ] || git clone --depth=1 --branch "$MYCELIUM_REF" \
      "https://github.com/$MYCELIUM_REPO.git" mycelium-src; \
    cd mycelium-src && git fetch && git checkout "$MYCELIUM_REF" && \
    cargo build --release --target "$ARCH_RUST" -p "$bin" ) \
    || die "local build of $bin failed"
  install -m 0755 "/tmp/mycelium-src/target/$ARCH_RUST/release/$bin" "$out"
}

fetch_binary mycelium-core         "$PREFIX/bin/mycelium-core"
fetch_binary mycelium-tool-runner  "$PREFIX/bin/mycelium-tool-runner"

# ── systemd units ───────────────────────────────────────────────────────
fetch_text() {
  local path="$1" out="$2"
  curl -fsSL "https://raw.githubusercontent.com/$MYCELIUM_REPO/$MYCELIUM_REF/$path" -o "$out" \
    || die "fetch $path"
}

mkdir -p /etc/systemd/system
fetch_text infra/systemd/mycelium-core.service /etc/systemd/system/mycelium-core.service
fetch_text infra/systemd/mycelium-tool-runner.service /etc/systemd/system/mycelium-tool-runner.service

# ── env files ───────────────────────────────────────────────────────────
mkdir -p /etc/mycelium
if [ ! -f /etc/mycelium/mycelium-core.env ]; then
  cat > /etc/mycelium/mycelium-core.env <<'EOF'
# Optional env overrides for mycelium-core. Anything in secrets.env
# (TELEGRAM_BOT_TOKEN, LLM_*) wins on top of these.
NATS_URL=nats://127.0.0.1:4222
RUST_LOG=info,mycelium_core=debug
EOF
  chmod 0644 /etc/mycelium/mycelium-core.env
fi
if [ ! -f /etc/mycelium/mycelium-tool-runner.env ]; then
  cat > /etc/mycelium/mycelium-tool-runner.env <<'EOF'
# Operator allow-list of capability strings (comma-separated).
# Skills declaring anything outside this set are refused at load time.
MYCELIUM_APPROVED_CAPS=wasi:clocks/wall-clock,wasi:clocks/monotonic-clock,wasi:io/streams,wasi:io/poll,wasi:io/error,wasi:http/outgoing-handler,wasi:http/types,wasi:logging/logging,wasi:random/random
MYCELIUM_SKILL_CACHE_DIR=/var/lib/mycelium/skill-cache
RUST_LOG=info,mycelium_tool_runner=debug
EOF
  chmod 0644 /etc/mycelium/mycelium-tool-runner.env
fi
[ -f "$SECRETS_FILE" ] || warn "$SECRETS_FILE missing; mycelium-core will not have Telegram + LLM creds"

# ── cut over ────────────────────────────────────────────────────────────
systemctl daemon-reload

if systemctl is-active --quiet mycelium-host.service; then
  log "stopping mycelium-host (wash)"
  systemctl disable --now mycelium-host.service || true
fi

systemctl enable  mycelium-core.service mycelium-tool-runner.service
systemctl restart mycelium-core.service mycelium-tool-runner.service

sleep 2
systemctl --no-pager --lines 0 status \
  mycelium-nats.service \
  mycelium-core.service \
  mycelium-tool-runner.service || true

log "done. tail logs: sudo journalctl -u mycelium-core -u mycelium-tool-runner -f"
