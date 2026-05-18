# 15 — Support team lead

**Persona**: Vera. Runs a 6-person support team. Bot does tier-1; humans do tier-2.
**Goal**: Bot handles 70% of tickets. Clean handoff for the rest. Audit + reporting.
**Primitives**: tier-1 agent + handoff event → human queue → reporting cron.

## Architecture

```
Telegram / web widget ──► gateway
                            │
                            ├── new ticket → mycelium.event.ticket.new
                            └── reply → conversation history
tier1-agent subscribes mycelium.task.step.agent
   on hard question → publish mycelium.event.handoff
   on resolved → publish mycelium.event.ticket.resolved

Human dashboard subscribes mycelium.event.handoff
   takes over via POST /conversations/<id>/messages (role=assistant)

Reporting cron daily 18:00 publishes a Slack/Telegram digest
```

## Happy path

```
22:14 Customer: "Where's my order?"
22:14 Bot:      [looks up via order-lookup tool] "Order #4521 shipped 17/05, arrives tomorrow."
22:15 Customer: "Thanks!"
22:15 Bot:      "Glad. Marked resolved."
```

## Branches

### A. Tiered fallback
- 3 turns no resolution → auto handoff
- Customer can opt-in to "always human" → KV flag on chat_id

### B. Knowledge base
- `memory/kb/<topic>` keys feed the agent context at step time
- Vera edits via simple web form

### C. Macros for humans
- `/macro greeting` expands into a templated message
- Stored in `memory/team/macros`

### D. Sentiment routing
- Angry customers (sentiment < threshold) route directly to human queue
- Even at tier 1

### E. SLA timers
- Each ticket has SLA in `memory/ticket/<id>/sla`
- cron-scheduler emits warnings as SLA approaches breach

### F. Reporting
- Daily digest: resolved count, avg response time, top topics, NPS
- Reportable to Slack / email / Telegram channel

### G. Multi-language
- Customer in German → routed to German-speaking human if handoff needed

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Customer waits forever | no human watching queue | dead-man-switch SMS to lead |
| Wrong tier-1 answer | KB stale | quick edit via web form |
| Handoff loop | both bot and human reply | role guard: bot stops on handoff event |
| Audit gaps | events not journaled | event-logger health alert |

## Connects to

- [End: customer support](end-customer-support.md)
- [Op: cron scheduler](op-cron-scheduler.md) — SLA + reporting
- [End: privacy](end-privacy.md) — SAR handling
