# 14 — Scheduled-digest subscriber

**Persona**: Tomas. Wants a Monday 8am summary of his journal from the past week.
**Goal**: Set it once, forget. Land in Telegram every Monday.
**Primitives**: `mycelium:cron/scheduler` (WIT). Cron job publishes payload to a NATS subject; agent consumes; reply via telegram-out.

## Happy path

Once:
```
Tomas:  /schedule weekly Monday 8:00 "summarize my journal last 7 days"
Bot:    Got it. Cron id MN-08. Next fire: 2026-05-25 08:00:00 Europe/Budapest.
        /schedule list to see all, /schedule delete MN-08 to cancel.
```

Every Monday 8am:
1. cron-scheduler component fires the registered job
2. Publishes `mycelium.task.submit` with `input` interpolated from the cron payload + current date range
3. Agent reads `memory/tomas/journal/*` keys for the last 7 days
4. Produces a summary
5. telegram-out sends it to Tomas

## Branches

### A. Multiple schedules
- "/schedule daily 22:00 reflect" + "/schedule monthly 1st 9:00 month-review"
- All live in `mycelium-cron-jobs` KV.

### B. Time zone aware
- Bot detects user TZ from first /start hint, stores in `memory/tomas/tz`.
- cron-scheduler converts to UTC when computing next fire.

### C. Pause / resume
- "/schedule pause MN-08" → `paused=true` on the cron job, scheduler skips it.
- "/schedule resume MN-08" → re-enable.

### D. Skip one occurrence
- "/schedule skip-next MN-08" → mark next fire as skipped.

### E. Failure recovery
- If the agent fails on a fire, retry-with-backoff inside cron-scheduler.
- After 3 fails, message the user with the error summary.

### F. Confirmation before send
- Optional "preview mode": instead of auto-sending, the digest first lands as "ready, send? y/n".

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Digest never fires | cron-scheduler crashed | health check + auto-restart |
| Wrong time zone | DST or missing user TZ | normalise to IANA TZ |
| Empty digest | no journal entries in window | digest-bot replies "nothing to summarise this week" |
| Duplicate digest | scheduler restart caused double-fire | idempotency key per (cron_id, fire_ts) |

## Connects to

- [Daily journal](end-journal.md) — feeds this
- [Op: cron scheduler](op-cron-scheduler.md)
- [Build: cron scheduler](build-cron-scheduler.md)
