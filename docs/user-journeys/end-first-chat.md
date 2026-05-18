# 01 — First-time Telegram chatter

**Persona**: Mara. Found bot via friend. Never used an AI assistant.
**Goal**: Send first message, get useful reply, not feel lost.
**Primitives**: telegram-poller (long-poll) reads `/start` from Telegram → publishes `mycelium.channel.in` → telegram-out reply.

## Setup (operator side, once)

Operator gives Mara the bot URL: `t.me/mycelium_demo_bot`.

## Happy path

1. Mara opens chat, taps "Start".
2. telegram-poller's next `getUpdates` (within ~2s) picks up `/start`.
3. Bot replies with a welcome card listing 3 example prompts.
4. Mara taps one ("Help me plan dinner").
5. Within 4-6s a reply: "Sure — what's in your fridge?"

Under the hood:
- telegram-poller (Service component, runs forever in `mycelium-telegram-poll`) issues `getUpdates` long-polls every ~2s.
- Each update is published on `mycelium.channel.in` (NATS).
- Operator's onboarding agent receives a step.
- Reply goes back via `mycelium.channel.telegram.out.${CHAT}` → telegram-out → Telegram sendMessage.

## Branches

### A. Mara goes silent for 7 days
- Re-engagement message lands ("hey, still cooking dinner?").
- Implemented by a scheduled cron NATS publish; operator opts in.

### B. Mara sends a question the bot can't answer
- Bot replies: "I'm not sure. Want me to flag this for a human?"
- "Yes" → routes to support queue ([end-customer-support](end-customer-support.md)).

### C. Mara wants to start over
- `/reset` → conversation closed, new one starts.
- Old history still in KV (operator-side, for audit).

### D. Mara reports the bot
- Telegram-level. Operator gets notified, can `/agent delete <agent_id>` to pull it.

## Failure modes

| What Mara sees | Cause | What operator does |
|---|---|---|
| No reply ever | telegram-out down | `just status` → restart `mycelium-telegram-out` |
| "Internal error" | LLM endpoint unreachable | check `llm.endpoint` in agent's KV value |
| Reply 30s late | model too slow | swap to faster model via KV |
| Wrong language | system_prompt missing lang hint | update agent config |

## Connects to

- [Family group](end-family-group.md) — same bot, different chat shape
- [Customer support](end-customer-support.md) — escalation
- [Op: first bot](op-first-bot.md) — what the operator did
