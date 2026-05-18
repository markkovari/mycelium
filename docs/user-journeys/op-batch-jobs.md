# 19 — Batch ops admin

**Persona**: Eva. Runs a content moderation team. Needs nightly run over 50k user posts.
**Goal**: Submit at 23:00, results in KV / S3 by 03:00, alert on stragglers.
**Primitives**: `mycelium:batch` enqueue + fan-out + result merge + alerting.

## Happy path

```bash
# Triggered nightly by cron-scheduler (or external CI)
curl -X POST http://api.internal:8080/batches -H 'host: internal.localhost' \
  -d @posts-2026-05-17.json
# → {"batch_id":"ME-2026-05-17","items":50000,"estimated_minutes":180}

# Live progress
nats sub 'mycelium.batch.ME-2026-05-17.progress' --count 1
# {"done":48732,"failed":83,"running":1185,"eta_min":7}

# Completion event
nats sub 'mycelium.batch.ME-2026-05-17.done' --count 1
# {"succeeded":49917,"failed":83,"results_kv":"mycelium-batch-results"}
```

## Branches

### A. Throughput tuning
- Bump `AGENT_POOL_SIZE=32` on `mycelium-agent` workload
- Or: run multiple wash hosts; NATS queue group balances
- Or: split agent into N specialised workloads (`mycelium-agent-fast`, `mycelium-agent-careful`)

### B. Budget gate
- Dispatcher reads `cfg("batch.budget_usd")` and emits warning event if projected cost > budget
- Operator approves via `nats pub mycelium.batch.<id>.continue`

### C. Sampled QA
- Dispatcher samples 1% to a separate `qa-checker` agent
- Mismatches above threshold → alert; pause batch

### D. Replay from history
- Yesterday's batch had a bad prompt. Operator updates agent KV. Re-runs same input file:
- `POST /batches { batch_id: "ME-2026-05-17-rerun", from: "ME-2026-05-17" }`

### E. Partition by tenant
- Each batch carries `tenant_id` in annotations
- Router fans tenant-specific events to tenant-specific subjects (`mycelium.event.tenant.<id>`)

### F. Failure burst handling
- If `failed/done > 0.05` mid-batch → auto-pause
- Operator gets paged on `mycelium.event.batch.unhealthy`

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| 1000 items stuck | upstream LLM 429 | exponential backoff already wired |
| Disk fills | KV results too large | rotate results to object store, KV holds index only |
| Stragglers from one tenant | hot key | per-tenant rate-limit in dispatcher |
| Lost batch state on host restart | KV is persistent (file storage), survives | verify `init-streams.sh` ran with file storage |

## Connects to

- [End: batch](end-batch.md) — same primitive, smaller scale
- [Build: batch pipeline](build-batch-pipeline.md)
- [Op: cron scheduler](op-cron-scheduler.md) — nightly trigger source
