# 08 — Daily journaler

**Persona**: Domi. 22:00 every night, the bot pings: "How was today?"
**Goal**: 2-min reflection. Weekly summary on Sunday.
**Primitives**: cron-scheduler nightly fire → journal-bot → KV store + Sunday digest.

## Setup

```bash
# Schedule nightly prompt
curl -X POST http://localhost:8080/cron -H 'host: localhost' -d '{
  "id":"domi-journal","schedule":"0 22 * * *",
  "subject":"mycelium.channel.telegram.out.123456",
  "payload":"{\"recipient_id\":\"123456\",\"text\":\"How was today?\"}",
  "timezone":"Europe/Budapest"
}'

# Schedule Sunday digest
curl -X POST http://localhost:8080/cron -H 'host: localhost' -d '{
  "id":"domi-weekly","schedule":"0 9 * * 0",
  "subject":"mycelium.task.submit",
  "payload":"{\"agent_id\":\"journal-digest\",\"input\":\"summarize Domi last 7 days\"}",
  "timezone":"Europe/Budapest"
}'
```

## Happy path

```
22:00 — Bot: "How was today?"
Domi:  Tired. Did 5km run. Boss meeting tense.
Bot:   Noted. Mood: 🟡. One reframe: tense ≠ unsuccessful.
       See you tomorrow.

Sun 09:00 — Bot: "Week summary:
       - 5 entries
       - mood trend: ⬇️ Mon-Wed, ⬆️ Thu-Sat
       - top topic: work stress (3/5)
       - suggestion: protect Wed afternoons for deep work"
```

## Branches

### A. Mood scoring
- Each entry → light sentiment tag stored in `memory/domi/journal/<date>/mood`
- Weekly digest aggregates trend

### B. Skip a day
- "/skip today" → no prompt, no penalty
- Useful for travel or just bad days

### C. Multiple prompts per day
- "/journal add lunch reflection" → ad-hoc entry
- Tagged `kind: ad-hoc` vs `kind: nightly`

### D. Export
- "/export pdf" → triggers a batch that compiles last N entries into a PDF
- Useful for therapy or personal archive

### E. Encrypted at rest
- KV value is client-encrypted (user passphrase). Bot can't read content, just stores blob.
- Sunday digest doesn't work in this mode unless user provides passphrase in chat.

### F. Streak gamification
- "/streak" → "12 days running"
- Stored in `memory/domi/journal/streak`

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Prompt fires twice | scheduler duplicate | idempotency key in cron-history |
| No digest Sunday | scheduler offline | catch-up policy = latest |
| Entry lost | wrote during NATS hiccup | gateway should ack only after KV set |
| Mood always neutral | model weak at sentiment | swap to specialised tagger |

## Connects to

- [Scheduled digest](end-scheduled-digest.md)
- [Privacy](end-privacy.md)
- [Op: cron scheduler](op-cron-scheduler.md)
