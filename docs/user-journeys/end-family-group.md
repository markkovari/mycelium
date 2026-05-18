# 05 — Family group member

**Persona**: Anna, Pal, Eszter, kids Lili & Boti. Shared Telegram group.
**Goal**: One bot for everyone. Knows who's who. Doesn't reply to every message.
**Primitives**: Group chat + per-member memory + mention-only response.

## Setup

Operator adds bot to group, grants minimal permissions (read mentions, send messages).
Bot config:
```bash
nats kv put mycelium-agent-config agent/family '{
  "id":"family","name":"FamBot",
  "system_prompt":"Reply only when @-mentioned or when a question is direct. Address members by name. Stay short.",
  "model":"qwen2.5:0.5b","tools":["calendar","calculator"],"max_steps":3
}'
```

## Happy path

```
Anna:   "@fambot grocery list for the week?"
Bot:    "Last week you bought: milk×3, bread×2, ... want a copy?"

Pal:    "anyone seen the keys"   ← no mention, bot stays silent

Lili:   "@fambot when's mom's birthday"
Bot:    "May 24. 6 days away."
```

## Branches

### A. Per-member memory
- `memory/family/anna/grocery-history`, `memory/family/lili/...`
- Bot writes from chat events tagged by Telegram `from.id`

### B. Family-wide calendar
- Bot reads/writes `memory/family/calendar` (shared)
- Conflict detection: "tonight 7pm — but you said dentist at 7"

### C. Quiet hours
- "@fambot mute 22:00-07:00"
- Stored in `memory/family/quiet-hours`. cron-scheduler skips fires in window.

### D. Permission tiers
- Kids can't trigger high-cost actions (e.g. `/spend 50€`)
- Implemented as a check inside the agent's system prompt + a member-role mapping in `memory/family/roles`

### E. Private DM to one parent
- Anna DMs the bot privately about a surprise party
- Bot remembers it's private (`chat.type == "private"`), doesn't leak to group

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Bot replies to everything | system_prompt drift | reset via PUT /agents/family |
| Wrong member attributed | telegram from.id missing | parse update.message.from.id correctly |
| Calendar collision missed | `memory/family/calendar` stale | nightly resync cron |

## Connects to

- [Voice memo](end-voice-memo.md) — kids leave audio
- [Scheduled digest](end-scheduled-digest.md) — weekly family summary
- [Privacy](end-privacy.md) — kids' data
