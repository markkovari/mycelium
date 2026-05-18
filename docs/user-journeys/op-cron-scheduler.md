# 20 — Cron scheduler admin

**Persona**: Boglarka. Runs the company's reminders + nightly digest jobs.
**Goal**: Manage 40 cron-like schedules across teams. Audit fires. Pause/skip.
**Primitives**: `mycelium:cron/scheduler` (WIT). KV-backed job table. NATS fire events.

## Happy path

```bash
# Register a job
curl -X POST http://internal:8080/cron -H 'host: internal.localhost' -d '{
  "id":"daily-standup-ping",
  "schedule":"0 9 * * 1-5",
  "subject":"mycelium.task.submit",
  "payload":"{\"agent_id\":\"standup\",\"input\":\"daily standup summary\"}",
  "timezone":"Europe/Budapest"
}'

# List
curl http://internal:8080/cron -H 'host: internal.localhost'

# History of fires
nats kv ls mycelium-cron-history -p 'fire/daily-standup-ping/*'
```

## Branches

### A. Per-team namespaces
- jobs keyed `cron/{team}/{id}` → ops UI filters by team
- ACL via NATS account subjects (`team-x.mycelium.cron.*`)

### B. Cron expression validator
- POST returns 400 if expression invalid; rejects "@yearly" if expansion ambiguous
- Friendly errors: "expected 5 fields, got 4"

### C. Dry-run preview
- `?dry_run=true` returns next 5 fire times without saving the job

### D. Drift / catch-up policy
- If scheduler was down 2h and missed 3 fires:
  - `catchup: "all"` → fire all 3 immediately (ordered)
  - `catchup: "latest"` → fire only the most recent
  - `catchup: "none"` → skip, log skipped fires

### E. Per-job retry policy
- `retry: { max: 3, backoff_sec: 60 }`
- After max retries, publish `mycelium.event.cron.dead-letter`

### F. One-shot scheduled tasks
- `schedule: "@once 2026-06-01T09:00:00Z"` → runs once, auto-deletes
- Used for: "remind me in 3 hours" UX from chat

### G. Webhook callout instead of NATS
- Some jobs need to hit an external URL: `subject: null, webhook: "https://api.x"`
- Cron component does the outgoing call via wasi:http

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Job doesn't fire | wrong TZ in expression | dry-run preview shows next 5 |
| Double fire | retry policy too eager | idempotency key per (job_id, fire_ts) |
| All jobs stop | cron component crashed | health check + auto-restart |
| Drift after long downtime | clock skew | sync host clock; catchup policy explicit |

## Connects to

- [End: scheduled digest](end-scheduled-digest.md) — end-user surface
- [Build: cron scheduler](build-cron-scheduler.md) — wire it
- [Op: batch jobs](op-batch-jobs.md) — nightly batches triggered from here
- [SRE oncall](op-sre-oncall.md) — alerts when jobs die
