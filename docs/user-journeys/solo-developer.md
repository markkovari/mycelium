# 01 — Solo developer

**Persona**: Indie dev. Building an AI side project. Wants tight inner loop.
**Goal**: Spin stack, register agent, iterate on prompt, ship to ghcr.
**Primitives**: `just dev`, `POST /agents`, `wash oci push`, GitHub Actions.

## Setup

```bash
git clone git@github.com:markkovari/mycelium.git && cd mycelium
just dev    # NATS + OCI registry + wash host + 10 workloads
```

Wait ~30s. Check status:
```bash
just status   # all 10 RUNNING
curl -H 'host: localhost' http://localhost:8080/health
```

## Happy path

```bash
# Register agent
curl -X POST http://localhost:8080/agents -H 'host: localhost' \
  -d '{"id":"draft","name":"Draft","system_prompt":"Be terse.","model":"qwen2.5:0.5b","tools":[],"max_steps":4}'

# Start a conversation
CID=$(curl -sX POST http://localhost:8080/conversations -H 'host: localhost' \
  -d '{"agent_id":"draft"}' | jq -r .id)

# Send a message
curl -X POST http://localhost:8080/conversations/$CID/messages -H 'host: localhost' \
  -d '{"role":"user","content":"explain CRDTs in 3 sentences"}'

# Trigger the step manually (auto-trigger blocked by v2 HTTP context limitation)
nats pub mycelium.task.submit "$(cat <<EOF
{"id":"t1","conversation_id":"$CID","agent_id":"draft","input":"explain CRDTs in 3 sentences","created_at":"now"}
EOF
)"

# Watch the reply land
nats sub mycelium.step.result --count 1
```

## Branches

### A. Iterate prompt without redeploy
```bash
# Just update KV — agent reads it on next step
curl -X PUT http://localhost:8080/agents/draft -H 'host: localhost' \
  -d '{"id":"draft","system_prompt":"Be terse and witty.","model":"qwen2.5:0.5b","tools":[],"max_steps":4}'
```
No rebuild. No push. Next `mycelium.task.submit` uses the new prompt.

### B. Swap to a stronger model mid-session
```bash
nats kv get mycelium-agent-config agent/draft \
  | jq '.model="llama3:8b"' \
  | nats kv put mycelium-agent-config agent/draft
```
Useful for: cheap model on draft → strong model on final pass.

### C. Add a custom LLM endpoint (Anthropic, OpenAI, local Ollama)
KV payload supports `endpoint` and `api_key` fields (off the WIT record):
```bash
nats kv put mycelium-agent-config agent/draft '{"id":"draft","name":"Draft","system_prompt":"Be terse.","model":"claude-3-5-sonnet-latest","tools":[],"max_steps":4,"endpoint":"https://api.anthropic.com/v1/messages","api_key":"sk-..."}'
```

### D. Hot-rebuild a single component
```bash
just build-one gateway
wash oci push --insecure localhost:5001/mycelium/gateway:dev target/wasm32-wasip2/release/gateway.wasm
PULL_POLICY=IMAGE_PULL_POLICY_ALWAYS just undeploy && just deploy
```
~5s cycle.

### E. Ship to ghcr.io
```bash
git push origin feat/new-agent
# CI matrix in .github/workflows/release.yaml pushes each component to
# ghcr.io/<user>/mycelium/<comp>:<sha> + :latest on main.
OCI_REGISTRY=ghcr.io/markkovari/mycelium IMAGE_TAG=latest just deploy
```

## Failure modes

| Symptom | Likely cause | Fix |
|---|---|---|
| `connection refused :8080` | host not running | `just status`, restart `just up` |
| `WORKLOAD_STATE_ERROR` on api | gateway image mismatch | `PULL_POLICY=IMAGE_PULL_POLICY_ALWAYS just deploy` |
| `POST /agents` returns 500 | KV bucket missing | `just init-streams` |
| Step never fires | forgot to `nats pub mycelium.task.submit` | publish manually (see Happy path) |
| Agent uses wrong model | KV not refreshed | `nats kv get mycelium-agent-config agent/<id>` to verify |

## Connects to

- [Content team](content-team.md) — same iteration loop, multiple agents
- [SaaS embed](saas-embed.md) — same REST surface, embedded elsewhere
- [Data analyst](data-analyst.md) — A/B model swap via KV
