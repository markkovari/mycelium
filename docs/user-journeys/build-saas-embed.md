# 20 — SaaS founder embedding mycelium

**Persona**: Liz. Building a project-management SaaS. Wants AI features without buying OpenAI directly.
**Goal**: Use mycelium's REST API behind her app. Her users never see "mycelium".
**Primitives**: REST `/agents` + `/conversations` from server-side. Multi-tenant via agent_id namespacing.

## Integration shape

```
Liz's frontend (React) ──► her API (Rust/Node) ──► mycelium-api REST
                                                     │
                                                     ├── POST /agents (one per tenant + use-case)
                                                     ├── POST /conversations
                                                     └── POST /conversations/:id/messages
```

Mycelium hides behind Liz's API. Customers see Liz's branding.

## Multi-tenant model

- Per-tenant agent ids: `agent/<tenant>/draft`, `agent/<tenant>/summarize`
- Conversation ids include tenant prefix
- Liz's API enforces auth and rewrites paths

## Branches

### A. Per-tenant model & key
- Each tenant chooses a model in their settings
- Liz's API updates `agent/<tenant>/<usecase>` KV with their preferred model + their own API key
- Cost falls on the tenant

### B. Quotas
- Liz's gateway proxy counts requests per tenant
- Hard cap at plan limit → her API replies 402

### C. White-label Telegram bot per tenant
- Each tenant brings their own BotFather token
- Operator-side: separate `mycelium-telegram-in-<tenant>` workload, scoped wasi:config

### D. Streaming responses
- Liz uses Server-Sent Events on her API
- Backend polls `mycelium.step.result` via NATS and streams partials

### E. Webhooks for events
- Tenant can register a webhook
- Liz's gateway subscribes `mycelium.event.>` and forwards to tenant webhook

### F. Self-hosted appliance (large tenant)
- Liz ships a Docker compose with mycelium pinned at a known version
- Enterprise customer runs it on-prem, points their API at it

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Tenant data leak | shared agent or namespace | strict prefix discipline + tests |
| Cost runaway | one tenant abuses | quota enforced before reaching mycelium |
| Slow startup | wash host cold start on Liz's appliance | k8s with kept-warm pods |
| Vendor lock | tied to mycelium API shape | keep adapter thin so swap is one file |

## Connects to

- [Solo developer](solo-developer.md) — same iteration loop
- [Edge Pi](build-edge-pi.md) — appliance variant
- [Build: data analyst](build-data-analyst.md) — Liz can offer A/B
