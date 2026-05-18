# 27 — Batch pipeline builder

**Persona**: Imre. Wires the batch-dispatcher component that doesn't exist yet.
**Goal**: A new WASM component, runnable in its own workload, that turns one batch submission into N task submissions and coalesces results.
**Primitives**: WIT (`mycelium:batch`), `wasmcloud:messaging/consumer`, `wasi:keyvalue/store`.

## Architecture

```
POST /batches  ──gateway──WIT──► batch-registry (KV write)
                                       │
                                       └── publishes mycelium.batch.submit
                                                  ▼
                              batch-dispatcher workload
                                       │
                                       ├── reads batch from KV
                                       ├── fans out N×mycelium.task.submit
                                       └── records each item state in KV
                                                  ▼
                             agent workload(s) process per-item
                                                  ▼
                              mycelium.step.result ──► batch-coalescer
                                                          │
                                                          └── on N/N → mycelium.batch.<id>.done
```

## New components

### `batch-registry` (WIT-callable)
- Exports `mycelium:batch/registry` (need to add to `wit/batch.wit`)
- Imports `wasi:keyvalue/store`
- KV `mycelium-batches` key `batch/{id}` → BatchManifest
- Lives in `mycelium-api` workload alongside gateway

### `batch-dispatcher` (NATS-driven)
- Exports `wasmcloud:messaging/handler`
- Imports `wasmcloud:messaging/consumer` + `wasi:keyvalue/store`
- Subscribes `mycelium.batch.submit`
- Per item: publish `mycelium.task.submit` with `batch_meta: {id, index}`
- Writes per-item status to `mycelium-batch-state` KV

### `batch-coalescer` (NATS-driven)
- Subscribes `mycelium.step.result`
- For results with `batch_meta`, updates per-item state
- On N/N → publishes `mycelium.batch.<id>.done`

## Deploy

Three new workloads:
```bash
# infra/deploy-v2.sh — append:
"mycelium-batch-dispatcher;batch-dispatcher;batch-dispatcher;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer;1"
"mycelium-batch-coalescer;batch-coalescer;batch-coalescer;wasi:keyvalue:store|wasi:logging:logging|wasmcloud:messaging:consumer;1"
# batch-registry rides inside mycelium-api alongside gateway
```

## Branches

### A. Sharded dispatcher
- Multiple dispatcher hosts; NATS queue group `batch-dispatcher` distributes
- Each shard handles a slice of items

### B. Priority lanes
- High-priority items go to `mycelium.task.submit.priority`
- agent workload subscribes both, processes priority first

### C. DLQ + replay
- Failed items land in `mycelium-batch-dlq` KV
- Separate `/batches/<id>/replay-failed` endpoint re-queues them

### D. Streaming results
- Subscribe `mycelium.batch.<id>.item.>` for live SSE
- Wire into a frontend dashboard

## Testing

```bash
# Unit: tests/batch_test.rs covers the coalescer math
cargo test -p mycelium-tests

# Integration: bring up stack, run feature file
just dev
behave docs/user-journeys/build-batch-pipeline.feature
```

## Connects to

- [End: batch](end-batch.md)
- [Op: batch jobs](op-batch-jobs.md)
- [Build: cron scheduler](build-cron-scheduler.md) — nightly trigger of batches
