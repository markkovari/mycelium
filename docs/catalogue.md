# Mycelium Component Catalogue

WIT interfaces: `mycelium:tool/tool-provider`, `mycelium:mcp/mcp-provider`, `mycelium:hook/hook-provider`.  
All WASM components are deny-by-default — only imports declared in the component's `world` and approved by the runner's allow-list are reachable.

---

## Tools (`mycelium:skill/skill`)

Single-function components. Export `tool-provider.invoke`. Loaded by `mycelium-tool-runner`.

### Implemented

| Component | Imports | Description |
|---|---|---|
| `tool-calc` | _(none)_ | Arithmetic expression evaluator — pure compute |
| `tool-time` | `wasi:clocks/wall-clock` | Current UTC time, timezone formatting |
| `tool-web-fetch` | `wasi:http/outgoing-handler`, `wasi:io/*` | HTTP GET/POST, returns body as string |

### Planned

| Component | Imports needed | Description |
|---|---|---|
| `tool-shell` | _(none — runner injects result)_ | Run pre-approved shell commands via allowlist; runner executes natively, result injected back |
| `tool-fs-read` | `wasi:filesystem/preopens`, `wasi:filesystem/types` | Read files within a preopened directory sandbox |
| `tool-fs-write` | `wasi:filesystem/preopens`, `wasi:filesystem/types` | Write/append files within sandbox |
| `tool-json-path` | _(none)_ | JSONPath / jq-style queries over a JSON string |
| `tool-regex` | _(none)_ | Regex match / replace / extract |
| `tool-base64` | _(none)_ | Encode / decode base64 |
| `tool-uuid` | `wasi:random/random` | Generate UUIDs |
| `tool-hash` | _(none)_ | SHA-256 / MD5 / BLAKE3 over string input |
| `tool-template` | _(none)_ | Handlebars / Tera template rendering |
| `tool-diff` | _(none)_ | Unified diff between two strings |
| `tool-csv` | _(none)_ | Parse / query CSV; return JSON rows |
| `tool-yaml` | _(none)_ | YAML ↔ JSON conversion |
| `tool-markdown` | _(none)_ | Render Markdown to plain text or HTML |
| `tool-image-resize` | `wasi:http` (optional fetch) | Resize / crop images; input/output base64 |
| `tool-semver` | _(none)_ | Parse, compare, bump semver strings |
| `tool-nats-publish` | _(NATS import TBD — host-provided)_ | Publish a message to a NATS subject |

---

## MCP Servers (`mycelium:mcp/mcp-skill`)

Multi-tool components. Export `mcp-provider.list-tools` + `mcp-provider.call-tool`. Compatible with MCP protocol clients.

### Implemented

| Component | Imports | Tools exposed |
|---|---|---|
| `mcp-demo` | _(none)_ | Echo, reverse-string (demo / test) |
| `mcp-todo` | `wasi:keyvalue/store`, `wasi:clocks/wall-clock` | add-todo, list-todos, complete-todo, delete-todo |

### Planned

| Component | Imports needed | Tools exposed |
|---|---|---|
| `mcp-git` | `wasi:filesystem/preopens`, `wasi:http` | git-status, git-log, git-diff, git-commit, git-push |
| `mcp-github` | `wasi:http/outgoing-handler` | list-prs, get-pr, create-issue, add-comment, list-issues |
| `mcp-sqlite` | `wasi:filesystem/preopens` | query, execute, list-tables, describe-table |
| `mcp-postgres` | `wasi:http` (TCP via http tunnel) or host-provided socket | query, execute, list-tables |
| `mcp-nats-kv` | host-provided NATS import | kv-get, kv-put, kv-delete, kv-list |
| `mcp-files` | `wasi:filesystem/preopens` | read-file, write-file, list-dir, move, delete |
| `mcp-search` | `wasi:http/outgoing-handler` | web-search (Brave/Serper), summarize-results |
| `mcp-email` | `wasi:http/outgoing-handler` | send-email (SMTP relay or API), list-inbox |
| `mcp-calendar` | `wasi:http/outgoing-handler` | list-events, create-event, delete-event (CalDAV / Google) |
| `mcp-slack` | `wasi:http/outgoing-handler` | post-message, list-channels, get-thread |
| `mcp-telegram` | `wasi:http/outgoing-handler` | send-message, get-updates |
| `mcp-browser` | host-provided CDP/Playwright injection | navigate, click, fill, screenshot, get-text |
| `mcp-vector-store` | `wasi:http` or host-provided embedding | upsert, search, delete (wraps NATS KV + embeddings) |
| `mcp-code-exec` | host sandbox (separate process) | run-python, run-js, run-bash (isolated subprocess, result injected) |
| `mcp-aws` | `wasi:http/outgoing-handler` | s3-get, s3-put, invoke-lambda, describe-ec2 |
| `mcp-prometheus` | `wasi:http/outgoing-handler` | query, query-range, list-metrics |

---

## Hooks (`mycelium:hook/hook-provider`)

Event interceptors. Export `hook-provider.handle(event-name, payload-json)`. Can mutate payload or block pipeline actions by returning `{"block": true, "reason": "..."}`.

### Implemented

| Component | Imports | Events handled |
|---|---|---|
| `hook-trace` | _(none)_ | Any — logs event name + payload for debugging |

### Planned

| Component | Imports needed | Events / purpose |
|---|---|---|
| `hook-rate-limit` | `wasi:clocks/monotonic-clock`, `wasi:keyvalue` | `before-tool-call` — token-bucket rate limiting per agent |
| `hook-audit-log` | `wasi:http/outgoing-handler` or `wasi:filesystem` | All events — append to audit trail (file or HTTP sink) |
| `hook-pii-redact` | _(none)_ | `before-llm-call`, `after-llm-call` — strip PII from messages |
| `hook-content-filter` | _(none)_ | `before-llm-call` — block disallowed topics / prompt injection patterns |
| `hook-cost-guard` | `wasi:clocks/wall-clock` | `before-llm-call` — block if estimated token cost exceeds budget |
| `hook-retry` | `wasi:clocks/monotonic-clock` | `after-tool-call` — exponential backoff retry on transient errors |
| `hook-metrics` | `wasi:http/outgoing-handler` | All events — emit OpenTelemetry spans / Prometheus counters |
| `hook-secret-scan` | _(none)_ | `before-llm-call`, `after-tool-call` — detect and block accidental secret leakage |
| `hook-schema-validate` | _(none)_ | `before-tool-call` — validate tool arguments against JSON Schema |

---

## Agents

Agents are configurations referencing a model + system prompt + tool set. Not WASM components themselves — stored in `mycelium-agent-config` KV, managed via `mycelium:agent/agent-registry`.

### Planned agent profiles

| Agent ID | Model | Tools / MCP servers | Purpose |
|---|---|---|---|
| `assistant` | configurable (Ollama/Anthropic/OpenAI) | `tool-time`, `tool-web-fetch`, `mcp-search` | General-purpose chat assistant |
| `coder` | Sonnet / DeepSeek-Coder | `mcp-git`, `mcp-files`, `mcp-code-exec`, `tool-diff` | Code review, refactor, explain |
| `researcher` | Opus / GPT-4o | `mcp-search`, `tool-web-fetch`, `mcp-files`, `tool-markdown` | Deep research + document synthesis |
| `ops` | configurable | `mcp-prometheus`, `mcp-aws`, `mcp-nats-kv`, `tool-shell` | Infrastructure queries, runbooks |
| `data-analyst` | configurable | `mcp-sqlite`, `mcp-postgres`, `tool-csv`, `tool-json-path`, `mcp-code-exec` | SQL queries, data exploration |
| `secretary` | lightweight (Haiku / llama3) | `mcp-calendar`, `mcp-email`, `mcp-slack`, `tool-time` | Scheduling, drafting, notifications |
| `guardian` | fast local model | `hook-content-filter`, `hook-pii-redact`, `hook-secret-scan` | Safety layer — runs as hook chain, not standalone agent |

---

## Capability matrix

Quick reference: which WASI imports each tier is allowed to use.

| Import | skill | mcp-skill | hook |
|---|---|---|---|
| `wasi:clocks/*` | opt-in | opt-in | opt-in |
| `wasi:http/*` | opt-in | opt-in | opt-in |
| `wasi:io/*` | opt-in | opt-in | opt-in |
| `wasi:filesystem/*` | opt-in | opt-in | — |
| `wasi:keyvalue/*` | — | opt-in | opt-in |
| `wasi:random/*` | opt-in | opt-in | opt-in |
| `wasi:logging/*` | opt-in | — | opt-in |
| NATS publish | host-provided (future) | host-provided (future) | — |

All opt-in: component must declare import in its `world` AND operator must enable in runner allow-list.
