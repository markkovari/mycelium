# 04 — Customer support seeker

**Persona**: Karl. Bought a SaaS thing. Sync broken.
**Goal**: Self-serve fix, or fast escalation to a human.
**Primitives**: tier-1 agent → tier-2 escalation → human handoff via tagged event.

## Happy path

```
Karl:  Sync stopped working yesterday
Bot:   Sorry to hear. Quick checks first:
         1. Are you online?
         2. Last sync timestamp?
       Or type "human" to skip.
Karl:  yes / 2pm yesterday
Bot:   Try re-auth at <link>. Did that help?
Karl:  yes!
Bot:   Glad. Anything else?
```

## Branches

### A. Karl types "human" right away
- Conversation tagged `handoff=requested` via memory write.
- Operator's dashboard polls `mycelium.event.handoff` (router rule fans out).
- Human agent posts via `POST /conversations/$CID/messages` with `role:"assistant"`.
- telegram-out picks up reply.

### B. Tier-1 can't resolve in 3 turns
- Agent's `max_steps: 3`. After 3, auto-escalate. Same handoff event.

### C. Karl was rude
- Detection by content filter (could be a separate moderator agent).
- Doesn't matter for v0; mention for v1 roadmap.

### D. Karl returns 2 weeks later, new issue
- Same `chat_id` → looks up `mycelium-channel-sessions` for prior conversation context.
- Bot opens with "welcome back, last time was sync — different issue?"

### E. Karl asks for data deletion
- Routes to [end-privacy](end-privacy.md).

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Loop of "did that help?" | agent missing exit step | reduce max_steps |
| Handoff never picked up | no consumer on mycelium.event.handoff | operator subs that subject |
| Wrong account context | no auth on bot | gate behind /verify flow |

## Connects to

- [Support team operator](op-support-team.md)
- [Privacy](end-privacy.md)
- [Photo](end-photo.md) — user uploads screenshot of error
