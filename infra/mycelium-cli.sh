#!/usr/bin/env bash
# mycelium CLI — small wrapper for token/provider/agent operations.
#
# Installed to /usr/local/bin/mycelium by install.sh.
#
# Subcommands:
#   mycelium provider <gemini|openai|anthropic|ollama|custom> <api-key|url>
#                                          # writes llm.endpoint/model/api_key + restarts host
#   mycelium token telegram <bot-token>     # writes telegram.bot_token + restarts host
#   mycelium model <model-name>             # overrides default llm.model
#   mycelium restart                        # systemctl restart mycelium-host
#   mycelium status                         # service + workload state
#   mycelium logs [host|nats]               # journalctl -fu mycelium-<service>
#   mycelium agent create <id> [--name=N] [--prompt=P] [--model=M] [--tools=a,b]
#   mycelium agent list
#   mycelium agent delete <id>
#   mycelium chat <agent-id> <text>         # POST /tasks
#   mycelium config show                    # cat /etc/mycelium/host.env (redacted)

set -euo pipefail

CONF_DIR="${MYCELIUM_CONF_DIR:-/etc/mycelium}"
ENV_FILE="$CONF_DIR/host.env"            # systemd EnvironmentFile (shell-var keys only)
SECRETS_FILE="$CONF_DIR/secrets.env"     # consumed by deploy-v2.sh, NOT systemd
API="${MYCELIUM_API:-http://127.0.0.1:8080}"
HOSTHDR="${MYCELIUM_HOSTHDR:-localhost}"
NATS_URL="${NATS_URL:-nats://127.0.0.1:4222}"
GHCR_OWNER="${GHCR_OWNER:-markkovari}"
GHCR_REGISTRY="${GHCR_REGISTRY:-ghcr.io}"
IMAGE_TAG="${IMAGE_TAG:-dev}"
MYCELIUM_REF="${MYCELIUM_REF:-main}"

if [ "$(id -u)" -ne 0 ]; then SUDO="sudo"; else SUDO=""; fi

die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
log()  { printf '\033[1;36m[mycelium]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[mycelium]\033[0m %s\n' "$*" >&2; }

_kv_write() {
    # _kv_write <file> <key> <value>
    local file="$1" key="$2" val="$3"
    $SUDO mkdir -p "$(dirname "$file")"
    $SUDO touch "$file"
    $SUDO chmod 600 "$file"
    local tmp
    tmp=$($SUDO mktemp)
    $SUDO sh -c "grep -v '^${key}=' '$file' > '$tmp' || true; printf '%s=%s\n' '$key' '$val' >> '$tmp'; mv '$tmp' '$file'; chmod 600 '$file'"
}
set_secret() { _kv_write "$SECRETS_FILE" "$1" "$2"; }

redeploy() {
    log "Redeploying workloads (picks up new secrets without dropping NATS state)"
    local dep="/usr/local/share/mycelium/deploy-v2.sh"
    if [ ! -x "$dep" ]; then
        log "Fetching deploy-v2.sh from ref=${MYCELIUM_REF}"
        local tmp
        tmp=$(mktemp)
        curl -fsSL "https://raw.githubusercontent.com/${GHCR_OWNER}/mycelium/${MYCELIUM_REF}/infra/deploy-v2.sh" -o "$tmp" \
            || die "could not fetch deploy-v2.sh"
        $SUDO mkdir -p /usr/local/share/mycelium
        $SUDO install -m 0755 "$tmp" "$dep"
        rm -f "$tmp"
    fi
    # Run as root so secrets.env (mode 0600) can be sourced.
    $SUDO env \
        OCI_REGISTRY="${GHCR_REGISTRY}/${GHCR_OWNER}/mycelium" \
        IMAGE_TAG="$IMAGE_TAG" \
        NATS_URL="$NATS_URL" \
        MYCELIUM_SECRETS_FILE="$SECRETS_FILE" \
        bash "$dep"
}

cmd_provider() {
    local p="${1:-}" v="${2:-}"
    [ -z "$p" ] && die "usage: mycelium provider <gemini|openai|anthropic|ollama|custom> <key-or-url>"
    local endpoint model
    case "$p" in
        gemini)
            [ -z "$v" ] && die "gemini provider needs API key"
            endpoint="https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
            model="${MYCELIUM_MODEL:-gemini-2.5-flash-lite}"
            set_secret "LLM_ENDPOINT" "$endpoint"
            set_secret "LLM_MODEL" "$model"
            set_secret "LLM_API_KEY" "$v"
            ;;
        openai)
            [ -z "$v" ] && die "openai provider needs API key"
            endpoint="https://api.openai.com/v1/chat/completions"
            model="${MYCELIUM_MODEL:-gpt-4o-mini}"
            set_secret "LLM_ENDPOINT" "$endpoint"
            set_secret "LLM_MODEL" "$model"
            set_secret "LLM_API_KEY" "$v"
            ;;
        anthropic)
            [ -z "$v" ] && die "anthropic provider needs API key"
            endpoint="https://api.anthropic.com/v1/openai/v1/chat/completions"
            model="${MYCELIUM_MODEL:-claude-3-5-haiku-latest}"
            set_secret "LLM_ENDPOINT" "$endpoint"
            set_secret "LLM_MODEL" "$model"
            set_secret "LLM_API_KEY" "$v"
            ;;
        ollama)
            endpoint="${v:-http://127.0.0.1:11434/v1/chat/completions}"
            model="${MYCELIUM_MODEL:-qwen2.5:0.5b}"
            set_secret "LLM_ENDPOINT" "$endpoint"
            set_secret "LLM_MODEL" "$model"
            ;;
        custom)
            [ -z "$v" ] && die "custom provider needs full endpoint URL"
            set_secret "LLM_ENDPOINT" "$v"
            ;;
        *) die "unknown provider: $p" ;;
    esac
    log "Provider set: $p (endpoint=$endpoint model=$model)"
    redeploy
}

cmd_token() {
    local kind="${1:-}" val="${2:-}"
    [ -z "$kind" ] || [ -z "$val" ] && die "usage: mycelium token telegram <token>"
    case "$kind" in
        telegram) set_secret "TELEGRAM_BOT_TOKEN" "$val"; log "telegram bot token saved"; redeploy ;;
        *) die "unknown token kind: $kind" ;;
    esac
}

cmd_model() {
    local m="${1:-}"
    [ -z "$m" ] && die "usage: mycelium model <model-name>"
    set_secret "LLM_MODEL" "$m"
    log "LLM_MODEL = $m"
    redeploy
}

cmd_status() {
    $SUDO systemctl --no-pager --lines 0 status mycelium-nats mycelium-host || true
    echo "--- /health ---"
    curl -sf -H "host: $HOSTHDR" "$API/health" || echo "(no response)"
    echo
    echo "--- /agents ---"
    curl -sf -H "host: $HOSTHDR" "$API/agents" || echo "(no response)"
    echo
}

cmd_logs() {
    local svc="${1:-host}"
    $SUDO journalctl -fu "mycelium-${svc}"
}

cmd_agent() {
    local sub="${1:-}"; shift || true
    case "$sub" in
        create)
            local id="${1:-}"; shift || true
            [ -z "$id" ] && die "usage: mycelium agent create <id> [--name=N] [--prompt=P] [--model=M] [--tools=a,b]"
            local name="$id" prompt="You are mycelium agent $id." model="" tools="[]" max_steps=4
            for a in "$@"; do
                case "$a" in
                    --name=*)   name="${a#*=}" ;;
                    --prompt=*) prompt="${a#*=}" ;;
                    --model=*)  model="${a#*=}" ;;
                    --tools=*)  tools="[\"$(echo "${a#*=}" | sed 's/,/","/g')\"]" ;;
                    --max-steps=*) max_steps="${a#*=}" ;;
                    *) die "unknown flag: $a" ;;
                esac
            done
            local body
            body=$(printf '{"id":"%s","name":"%s","system_prompt":%s,"model":"%s","tools":%s,"max_steps":%s}' \
                "$id" "$name" "$(printf '%s' "$prompt" | python3 -c 'import sys,json;print(json.dumps(sys.stdin.read()))')" \
                "$model" "$tools" "$max_steps")
            curl -sf -X POST -H "host: $HOSTHDR" -H 'content-type: application/json' -d "$body" "$API/agents" \
                && echo
            ;;
        list) curl -sf -H "host: $HOSTHDR" "$API/agents" && echo ;;
        delete)
            local id="${1:-}"
            [ -z "$id" ] && die "usage: mycelium agent delete <id>"
            curl -sf -X DELETE -H "host: $HOSTHDR" "$API/agents/$id" && echo
            ;;
        *) die "agent subcommand: create|list|delete" ;;
    esac
}

cmd_chat() {
    local agent="${1:-}" text="${2:-}"
    [ -z "$agent" ] || [ -z "$text" ] && die "usage: mycelium chat <agent-id> <text>"
    command -v nats >/dev/null 2>&1 || die "nats CLI required (install: apt install unzip; see install.sh)"

    # Persist the conversation + user message via the gateway so future state
    # (memory store, history) can pick it up.
    local conv_body conv_resp conv_id
    conv_body=$(python3 -c "import json,sys;print(json.dumps({'agent_id':sys.argv[1]}))" "$agent")
    conv_resp=$(/usr/bin/curl -sf -X POST -H "host: $HOSTHDR" -H 'content-type: application/json' \
        -d "$conv_body" "$API/conversations") || die "POST /conversations failed"
    conv_id=$(python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])' <<<"$conv_resp")
    log "conversation $conv_id"

    local msg_body
    msg_body=$(python3 -c "import json,sys;print(json.dumps({'role':'user','content':sys.argv[1]}))" "$text")
    /usr/bin/curl -sf -X POST -H "host: $HOSTHDR" -H 'content-type: application/json' \
        -d "$msg_body" "$API/conversations/${conv_id}/messages" >/dev/null || die "POST /messages failed"

    # The gateway can't publish to NATS (HTTP plugin context limitation), so
    # the CLI triggers the executor directly.
    local task_id task_body
    task_id=$(python3 -c 'import uuid;print(uuid.uuid4())')
    task_body=$(python3 -c "
import json,sys,datetime
print(json.dumps({
    'id': sys.argv[1],
    'conversation_id': sys.argv[2],
    'agent_id': sys.argv[3],
    'input': sys.argv[4],
    'created_at': datetime.datetime.utcnow().isoformat()+'Z'
}))" "$task_id" "$conv_id" "$agent" "$text")

    log "publishing task $task_id → mycelium.task.submit"
    # Start subscriber BEFORE publishing to avoid missing the result.
    local sub_log
    sub_log=$(mktemp)
    nats --server="$NATS_URL" sub mycelium.step.result --count=1 --raw >"$sub_log" 2>/dev/null &
    local sub_pid=$!
    sleep 0.5
    nats --server="$NATS_URL" pub mycelium.task.submit "$task_body" >/dev/null

    log "waiting for mycelium.step.result (up to 30s)…"
    local i=0
    while [ $i -lt 30 ] && kill -0 $sub_pid 2>/dev/null; do
        sleep 1; i=$((i+1))
    done
    kill $sub_pid 2>/dev/null
    wait $sub_pid 2>/dev/null
    local raw
    raw=$(cat "$sub_log")
    rm -f "$sub_log"
    if [ -z "$raw" ]; then
        warn "no step.result within 30s; check: mycelium logs host"
        return 1
    fi
    echo
    echo "─── step result ───"
    printf '%s\n' "$raw" | python3 -m json.tool 2>/dev/null || printf '%s\n' "$raw"
}

cmd_config_show() {
    echo "# $SECRETS_FILE"
    [ -f "$SECRETS_FILE" ] && $SUDO sed -E 's/(API_KEY|BOT_TOKEN)=.*/\1=***redacted***/' "$SECRETS_FILE" || echo "(empty)"
    echo
    echo "# $ENV_FILE"
    [ -f "$ENV_FILE" ] && $SUDO cat "$ENV_FILE" || echo "(empty)"
}

# ── Onboarding wizard ────────────────────────────────────────────────────────
prompt() {
    # prompt VAR_NAME "question" "default"
    local var="$1" q="$2" def="${3:-}" ans
    if [ -n "$def" ]; then
        printf '%s [%s]: ' "$q" "$def" >&2
    else
        printf '%s: ' "$q" >&2
    fi
    IFS= read -r ans </dev/tty || ans=""
    [ -z "$ans" ] && ans="$def"
    printf -v "$var" '%s' "$ans"
}
prompt_secret() {
    # prompt_secret VAR_NAME "question"
    local var="$1" q="$2" ans
    printf '%s: ' "$q" >&2
    stty -echo </dev/tty
    IFS= read -r ans </dev/tty || ans=""
    stty echo </dev/tty
    printf '\n' >&2
    printf -v "$var" '%s' "$ans"
}
confirm() {
    # confirm "question" "default-yn"
    local q="$1" def="${2:-n}" ans
    case "$def" in y|Y) printf '%s [Y/n]: ' "$q" >&2 ;; *) printf '%s [y/N]: ' "$q" >&2 ;; esac
    IFS= read -r ans </dev/tty || ans=""
    [ -z "$ans" ] && ans="$def"
    case "$ans" in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

cmd_init() {
    [ -t 0 ] || die "mycelium init requires a TTY (pipe to bash will not work)"
    cat <<'BANNER'
┌──────────────────────────────────────────────────────────┐
│  mycelium onboarding                                     │
│  Walks you through LLM provider + Telegram token setup.  │
│  Existing values in /etc/mycelium/host.env are preserved │
│  unless you overwrite them here.                         │
└──────────────────────────────────────────────────────────┘
BANNER

    local provider
    while :; do
        prompt provider "LLM provider (gemini / openai / anthropic / ollama / skip)" "gemini"
        case "$provider" in gemini|openai|anthropic|ollama|skip) break ;; esac
        echo "  unknown: $provider" >&2
    done

    case "$provider" in
        gemini)
            local key model
            prompt_secret key "  Gemini API key"
            [ -z "$key" ] && die "key required"
            prompt model "  Model" "gemini-2.5-flash-lite"
            MYCELIUM_MODEL="$model" cmd_provider gemini "$key" >/dev/null
            log "Gemini provider configured (model=$model)"
            ;;
        openai)
            local key model
            prompt_secret key "  OpenAI API key"
            [ -z "$key" ] && die "key required"
            prompt model "  Model" "gpt-4o-mini"
            MYCELIUM_MODEL="$model" cmd_provider openai "$key" >/dev/null
            log "OpenAI provider configured (model=$model)"
            ;;
        anthropic)
            local key model
            prompt_secret key "  Anthropic API key"
            [ -z "$key" ] && die "key required"
            prompt model "  Model" "claude-3-5-haiku-latest"
            MYCELIUM_MODEL="$model" cmd_provider anthropic "$key" >/dev/null
            log "Anthropic provider configured (model=$model)"
            ;;
        ollama)
            local url model
            prompt url "  Ollama endpoint" "http://127.0.0.1:11434/v1/chat/completions"
            prompt model "  Model" "qwen2.5:0.5b"
            MYCELIUM_MODEL="$model" cmd_provider ollama "$url" >/dev/null
            log "Ollama provider configured ($url, model=$model)"
            ;;
        skip) log "Skipping LLM provider; agent calls will 401 until configured." ;;
    esac

    echo
    if confirm "Wire up Telegram bot now?" "n"; then
        local tok
        prompt_secret tok "  Telegram bot token (from @BotFather)"
        [ -n "$tok" ] && cmd_token telegram "$tok" >/dev/null && log "Telegram token saved"
    fi

    echo
    if confirm "Create a starter agent?" "y"; then
        local id name promptt
        prompt id "  Agent id" "alice"
        prompt name "  Display name" "$id"
        prompt promptt "  System prompt" "You are mycelium agent $id. Be concise."
        # Wait for /agents to come up after restart.
        for _ in 1 2 3 4 5; do
            curl -sf -H "host: $HOSTHDR" "$API/agents" >/dev/null 2>&1 && break
            sleep 1
        done
        cmd_agent create "$id" --name="$name" --prompt="$promptt" || warn "agent create failed; rerun: mycelium agent create $id"
    fi

    echo
    log "Done. Try: mycelium status   then   mycelium chat <agent-id> 'hello'"
}

usage() {
    cat <<'EOF'
mycelium — wash host control wrapper

Quickstart:
  mycelium init                        # interactive onboarding (LLM + telegram + agent)
  mycelium provider gemini AIza...     # or non-interactive
  mycelium token telegram 123:ABC
  mycelium agent create alice --prompt='Be terse.'
  mycelium chat alice 'hello'
  mycelium status
  mycelium logs host

Commands:
  init                                  interactive wizard
  provider <gemini|openai|anthropic|ollama|custom> <key-or-url>
  token telegram <token>
  model <model-name>
  status
  logs [host|nats]
  restart
  config show
  agent create <id> [--name=N --prompt=P --model=M --tools=a,b --max-steps=N]
  agent list
  agent delete <id>
  chat <agent-id> <text>
EOF
}

main() {
    local cmd="${1:-}"; shift || true
    case "$cmd" in
        init)         cmd_init ;;
        provider)     cmd_provider "$@" ;;
        token)        cmd_token "$@" ;;
        model)        cmd_model "$@" ;;
        status)       cmd_status ;;
        logs)         cmd_logs "$@" ;;
        restart)      log "Restarting mycelium-host (workloads will redeploy)"; $SUDO systemctl restart mycelium-host; sleep 3; redeploy ;;
        redeploy)     redeploy ;;
        agent)        cmd_agent "$@" ;;
        chat)         cmd_chat "$@" ;;
        config)
            local sub="${1:-}"; shift || true
            [ "$sub" = "show" ] && cmd_config_show || usage
            ;;
        ""|help|-h|--help) usage ;;
        *) usage; exit 1 ;;
    esac
}

main "$@"
