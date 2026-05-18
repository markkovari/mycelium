# 11 — Power switcher

**Persona**: Bence. Uses 6 different agents (researcher, planner, coach, code, draft, recipe).
**Goal**: Switch agents fast. No friction.
**Primitives**: `/agent <id>` Telegram command OR REST `?agent_id=` per message.

## Happy path

```
Bence:  /agent researcher
Bot:    [researcher persona engaged. tools: web-search, citations]

Bence:  Q3 EU AI Act impact on small SaaS?
Bot:    [grounded answer with 5 sources]

Bence:  /agent draft
Bot:    [draft persona. tools: none. low cost.]

Bence:  rewrite this as a tweet
Bot:    [tight, no citations]
```

## Branches

### A. Per-agent conversation
- Each agent has its OWN conversation by default
- `/agent <id>` switches the active one in `memory/<user>/active-agent`
- Old conversation persists, switchback continues from there

### B. Shared conversation
- "/share-context" pulls last 20 messages from another conversation into this one
- Useful: "researcher found X, now have draft turn it into a tweet"

### C. Hotkey aliases
- `/r` = researcher, `/d` = draft, `/c` = code
- Stored in `memory/<user>/aliases`

### D. Agent presets / chains
- "/chain research-then-draft" → researcher first, then draft uses researcher's output
- Implemented as a batch with a 2-item agent override list

### E. Cost-aware routing
- "/cheap" → forces fast-cheap model on next message
- "/strong" → forces high-end model
- Bypasses default `agent_id` model

### F. Multi-agent quorum
- "/ask-all q1 q2 q3" → 3 agents answer; bot returns all three replies for comparison
- Connects to [Data analyst](build-data-analyst.md)

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Active agent confused | /agent didn't update KV | confirm switch in reply |
| Lost context on switch | per-agent conversation | offer /share-context |
| Wrong alias | typo | suggestion ("did you mean /r?") |

## Connects to

- [Solo developer](solo-developer.md) — many agents, single user
- [Content pipeline](build-content-pipeline.md) — pre-defined chains
- [Data analyst](build-data-analyst.md) — multi-agent compare
