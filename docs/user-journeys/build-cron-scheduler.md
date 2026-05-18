# 28 — Cron scheduler builder

**Persona**: Mira. Implements the missing cron-scheduler component.
**Goal**: A WASM component that fires NATS publishes (or webhooks) on a cron-like schedule, with idempotency, catch-up policy, dead-letter.
**Primitives**: WIT (`mycelium:cron`), `wasi:clocks/wall-clock`, `wasi:keyvalue/store`, `wasmcloud:messaging/consumer`.

## Architecture

```
cron-scheduler workload (single instance)
   │
   │ every minute (sleep loop):
   │
   ├── list mycelium-cron-jobs KV
   ├── for each due job:
   │     ├── publish job.subject with job.payload (NATS)
   │     │     OR invoke webhook via wasi:http
   │     ├── write fire to mycelium-cron-history KV (idempotency)
   │     └── compute next_fire, persist
   │
   └── catch-up if last fire ts << now (policy-driven)
```

## WIT surface

Already defined in `wit/cron.wit`:
- `register(cron-job) -> result<string, string>`
- `unregister(id) -> result<_, string>`
- `list-jobs() -> list<cron-job>`

Add (this round):
- `pause(id) -> result<_, string>`
- `resume(id) -> result<_, string>`
- `skip-next(id) -> result<_, string>`
- `history(id, limit) -> result<list<fire-record>, string>`

## New component

### `cron-scheduler`
- Exports `mycelium:cron/scheduler` (WIT-callable from gateway for CRUD)
- Imports `wasmcloud:messaging/consumer` (for publishing fires)
- Imports `wasi:http/outgoing-handler` (for webhook-out variant)
- Imports `wasi:keyvalue/store` + `wasi:clocks/wall-clock`
- Background fire loop: `wasi:clocks` ticks every 60s
- Lives in own workload `mycelium-cron` (no HTTP exporter → can publish freely)

### `cron-admin` (HTTP front)
- Lives in `mycelium-api` alongside gateway
- Exports nothing of its own; gateway imports `mycelium:cron/scheduler` to mount CRUD routes
- This means: `POST /cron`, `GET /cron`, etc. land via WIT into cron-scheduler

## Deploy

```bash
# infra/deploy-v2.sh — append:
"mycelium-cron;cron;cron-scheduler;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer|wasi:http:outgoing-handler;1"
```

`mycelium-api` host_interfaces stay as-is (gateway calls cron-scheduler via WIT in same workload; need to add cron-scheduler to mycelium-api components list).

## Branches

### A. Distributed leader election
- Multiple cron-scheduler hosts; one is leader via NATS KV lease
- Failover < 30s

### B. Sub-minute precision
- Default tick = 60s. Bump to 1s tick for cron expressions with sub-minute granularity (none in standard cron; future feature).

### C. Cron expression library
- Use a pure-Rust crate (`cron` v0.x). Validate at register time.

### D. UI for ops
- A small React app talks to gateway's `/cron` routes
- Renders next-5-fires per job
- Drag-to-edit schedule (future feature)

### E. Test mode
- `?test_fire=true` on POST → immediately publishes the configured subject + payload once

## Testing

```bash
just build-one cron-scheduler
just push-oci
just deploy

# Smoke: create a 1-minute schedule, watch fires
curl -X POST http://localhost:8080/cron -H 'host: localhost' \
  -d '{"id":"t1","schedule":"* * * * *","subject":"test.echo","payload":"hello"}'

nats sub 'test.echo' --count 3
# wait 3 minutes, 3 messages arrive
```

## Connects to

- [End: scheduled digest](end-scheduled-digest.md)
- [Op: cron scheduler](op-cron-scheduler.md)
- [Build: batch pipeline](build-batch-pipeline.md) — nightly triggers
