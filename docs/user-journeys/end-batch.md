# 13 — Batched-action user

**Persona**: Anett. Marketing. Has 200 customer emails to rewrite in a friendlier tone.
**Goal**: One ask, watch progress, get all rewritten back.
**Primitives**: `mycelium:batch/batch-job` (WIT) → fans out one task per item → per-item NATS reply on `mycelium.batch.<batch-id>.item.<i>.result`.

## Happy path

```
Anett:  /batch rewrite friendly
         [attaches emails.txt — 200 lines, one email per line]
Bot:    Got 200 items. Estimated 4 minutes at 8 parallel.
        I'll DM you when done. Track with /batch status XK3F9.

[3.5 min later]
Bot:    Done. 198/200 succeeded.
         Download: <link>
         2 failures: lines 47, 132 (see /batch errors XK3F9).
```

Wire (current implementation hint — `mycelium:batch` WIT defined, dispatcher component is a future build target):
- gateway accepts `POST /batches { agent_id, items: [...] }` → returns batch id
- batch-dispatcher publishes N `mycelium.task.submit` with shared batch tag
- each step.result is captured in `mycelium-batch-results` KV (key `batch/{id}/item/{i}`)
- a coalescing worker watches and emits a single completion event on `mycelium.batch.<id>.done`

## Branches

### A. Cancel mid-batch
- `/batch cancel XK3F9` → publishes `mycelium.batch.<id>.cancel`
- Dispatcher stops pulling new items. In-flight items still complete.

### B. Resume after disconnect
- Anett closes Telegram, reconnects 1h later.
- `/batch status XK3F9` reads `mycelium-batch-results` KV.

### C. Per-item retry
- 2 failed. `/batch retry XK3F9 47 132` → republishes only those two.

### D. Different agent per item
- Power-user mode: items themselves carry `agent_id` overrides.
- Useful for: translation batch where each item has a target language.

### E. Progress streaming
- WebSocket-style polling: GET `/batches/<id>/stream` (SSE) → frontend shows live count.
- For Telegram, edits a single message every 5s with `n/200 done`.

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Stuck at 198/200 | item poison-pills agent | timeout per item, mark failed |
| Cost runaway | model spend × 200 | budget gate in dispatcher |
| Out-of-order results | OK by design | result merge is keyed by item index |
| Rate-limited by LLM | upstream 429 | exponential backoff in batch-dispatcher |

## Connects to

- [Op: batch jobs](op-batch-jobs.md) — operator scaling 200 → 200k
- [Build: batch pipeline](build-batch-pipeline.md) — wire it up
- [Content team](build-content-pipeline.md) — same fan-out, different prompts
