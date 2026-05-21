#!/usr/bin/env bash
# Mycelium one-shot installer for Linux (aarch64 / x86_64).
#
# Sets up NATS + JetStream, downloads the mycelium-core + mycelium-tool-runner
# binaries, installs the systemd units, and starts everything.
#
# Usage (Pi):
#   curl -fsSL https://raw.githubusercontent.com/markkovari/mycelium/main/infra/install.sh | sudo bash
#
# Or from a checkout:
#   sudo bash infra/install.sh
#
# Idempotent. Rerun to upgrade.

set -euo pipefail

# ── Defaults ────────────────────────────────────────────────────────────
GHCR_OWNER="${GHCR_OWNER:-markkovari}"
MYCELIUM_REPO="${MYCELIUM_REPO:-${GHCR_OWNER}/mycelium}"
MYCELIUM_REF="${MYCELIUM_REF:-main}"
PREFIX="${PREFIX:-/usr/local}"
STATE_DIR="${STATE_DIR:-/var/lib/mycelium}"
CONF_DIR="${CONF_DIR:-/etc/mycelium}"
NATS_VERSION="${NATS_VERSION:-2.10.20}"
NATS_CLI_VERSION="${NATS_CLI_VERSION:-0.1.5}"
SKIP_NATS_INSTALL="${SKIP_NATS_INSTALL:-0}"

log()  { printf '\033[1;36m[install]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[install]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[install]\033[0m %s\n' "$*" >&2; exit 1; }

[ "$EUID" -eq 0 ] || die "run as root (sudo)"

# ── Pre-flight ──────────────────────────────────────────────────────────
log "Mycelium installer starting"
case "$(uname -s)" in Linux) ;; *) die "Linux only" ;; esac
case "$(uname -m)" in
  aarch64|arm64) ARCH_TAR=arm64 ;;
  x86_64|amd64)  ARCH_TAR=amd64 ;;
  *) die "unsupported arch: $(uname -m)" ;;
esac

REPO_RAW="https://raw.githubusercontent.com/${MYCELIUM_REPO}/${MYCELIUM_REF}"

# ── NATS server + CLI ───────────────────────────────────────────────────
if [ "$SKIP_NATS_INSTALL" != "1" ]; then
  if ! command -v nats-server >/dev/null 2>&1; then
    log "Installing nats-server $NATS_VERSION"
    tmp=$(mktemp -d)
    url="https://github.com/nats-io/nats-server/releases/download/v${NATS_VERSION}/nats-server-v${NATS_VERSION}-linux-${ARCH_TAR}.tar.gz"
    curl -fsSL "$url" | tar -xz -C "$tmp"
    install -m 0755 "$tmp"/nats-server-*/nats-server "$PREFIX/bin/nats-server"
    rm -rf "$tmp"
  else
    log "nats-server: $(command -v nats-server)"
  fi
  if ! command -v nats >/dev/null 2>&1; then
    log "Installing nats CLI $NATS_CLI_VERSION"
    tmp=$(mktemp -d)
    url="https://github.com/nats-io/natscli/releases/download/v${NATS_CLI_VERSION}/nats-${NATS_CLI_VERSION}-linux-${ARCH_TAR}.zip"
    if curl -fsSL "$url" -o "$tmp/nats.zip" && command -v unzip >/dev/null 2>&1; then
      unzip -q "$tmp/nats.zip" -d "$tmp"
      install -m 0755 "$tmp"/nats-*/nats "$PREFIX/bin/nats"
    else
      warn "  nats CLI install skipped (install unzip and rerun)"
    fi
    rm -rf "$tmp"
  fi
fi

# ── Dirs + conf ─────────────────────────────────────────────────────────
mkdir -p "$STATE_DIR" "$STATE_DIR/skill-cache" "$CONF_DIR"
if ! id mycelium >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin mycelium
fi
chown -R mycelium:mycelium "$STATE_DIR"

if [ ! -f "$CONF_DIR/secrets.env" ]; then
  cat > "$CONF_DIR/secrets.env" <<'EOF'
# Populate via `mycelium init` or by editing. Mode 0600.
# LLM_ENDPOINT=https://generativelanguage.googleapis.com/v1beta/openai/chat/completions
# LLM_MODEL=gemini-2.5-flash-lite
# LLM_API_KEY=
# TELEGRAM_BOT_TOKEN=
# LLM_RPM=10
# LLM_RPD=200
EOF
  chmod 0600 "$CONF_DIR/secrets.env"
fi

# ── NATS systemd unit ───────────────────────────────────────────────────
if command -v systemctl >/dev/null 2>&1; then
  log "Installing mycelium-nats.service"
  curl -fsSL "${REPO_RAW}/infra/systemd/mycelium-nats.service" \
    -o /etc/systemd/system/mycelium-nats.service \
    || warn "could not fetch mycelium-nats.service unit; assuming locally present"
  systemctl daemon-reload
  systemctl enable --now mycelium-nats.service
fi

# ── Bootstrap streams + KV ──────────────────────────────────────────────
if command -v nats >/dev/null 2>&1; then
  log "Initialising JetStream streams + KV buckets"
  tmp=$(mktemp)
  curl -fsSL "${REPO_RAW}/infra/init-streams.sh" -o "$tmp"
  NATS_URL="nats://127.0.0.1:4222" bash "$tmp" \
    || warn "init-streams.sh failed; rerun manually"
  rm -f "$tmp"
fi

# ── Native binaries + units (delegate to deploy-native.sh) ──────────────
log "Installing mycelium-core + mycelium-tool-runner via deploy-native.sh"
tmp_dep=$(mktemp)
curl -fsSL "${REPO_RAW}/infra/deploy-native.sh" -o "$tmp_dep"
GHCR_OWNER="$GHCR_OWNER" MYCELIUM_REPO="$MYCELIUM_REPO" MYCELIUM_REF="$MYCELIUM_REF" \
  PREFIX="$PREFIX" STATE_DIR="$STATE_DIR" SECRETS_FILE="$CONF_DIR/secrets.env" \
  bash "$tmp_dep"
rm -f "$tmp_dep"
install -m 0755 "$tmp_dep" /usr/local/share/mycelium/deploy-native.sh 2>/dev/null || true

# ── mycelium CLI ────────────────────────────────────────────────────────
log "Installing mycelium CLI wrapper"
tmp_cli=$(mktemp)
curl -fsSL "${REPO_RAW}/infra/mycelium-cli.sh" -o "$tmp_cli"
install -m 0755 "$tmp_cli" "$PREFIX/bin/mycelium"
rm -f "$tmp_cli"

cat <<EOF

Mycelium installed.

  config:   $CONF_DIR/secrets.env
  data:     $STATE_DIR
  cli:      $PREFIX/bin/mycelium

next steps:
  sudo $PREFIX/bin/mycelium init       # populate Telegram + LLM secrets
  sudo systemctl status mycelium-core mycelium-tool-runner
  sudo journalctl -u mycelium-core -u mycelium-tool-runner -f

EOF
