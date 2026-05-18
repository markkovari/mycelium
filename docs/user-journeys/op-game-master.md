# 16 — Game master

**Persona**: Andras. Runs a D&D campaign with 5 players on Telegram between sessions.
**Goal**: 8+ NPCs, each with distinct voice + memory. Players DM the NPC they want.
**Primitives**: One agent per NPC + per-player memory + lore tool.

## NPCs

```bash
for npc in elara graff thissa zol; do
  curl -X POST http://localhost:8080/agents -H 'host: localhost' -d "{
    \"id\":\"npc-$npc\",
    \"name\":\"$npc\",
    \"system_prompt\":\"[full character sheet here]\",
    \"model\":\"qwen2.5:0.5b\",
    \"tools\":[\"lore-lookup\",\"dice\"],
    \"max_steps\":5
  }"
done
```

## Happy path

```
Player Tom DMs the bot:
  /npc elara
  "Greetings — what do you know of the broken seal?"

Bot (as Elara, sage):
  "Few speak of it. I heard whispers in the deep library —
   the seal was forged by Threll the Bound. Be cautious,
   the markings echo in dreams."

Andras (GM) in his admin chat:
  /lore add seal "forged by Threll, echoes in dreams"
  → updates memory/campaign/seal in the lore tool's KV
```

## Branches

### A. Cross-NPC consistency
- Lore tool reads `memory/campaign/<topic>` → every NPC says compatible things
- If updated by GM, all NPCs see the update next turn

### B. Per-NPC memory of each player
- `memory/npc-elara/player-tom/relations` = "trusts somewhat, owes a favor"
- NPC personalises replies

### C. Dice / random
- Player says "I attempt to persuade her"
- NPC's tool call: `dice` returns `{roll: 14, modifier: +2, total: 16}`
- NPC narrates outcome respecting roll

### D. Cliffhanger pacing
- GM publishes `mycelium.event.session.starts` → all NPCs go silent until end
- After session, GM publishes `mycelium.event.session.ends` → NPCs reactivate

### E. Recap on session day
- cron at session-start-1h → all players get "previously on Avernath…" generated from campaign log

### F. Banishment / death
- `/npc-archive elara` → marks NPC as dormant; new DMs to them say "they are gone"

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| NPCs contradict each other | lore not centralised | every NPC reads lore tool |
| NPC out of character | system_prompt too short | beef up character sheet |
| Bot reveals dice roll number | leakage in prompt | strict reveal rules in system_prompt |
| Player abuses by DMing 30 NPCs | rate-limit per chat_id | dispatcher throttle |

## Connects to

- [Power switch](end-power-switch.md) — same /agent switching pattern
- [Op: teacher](op-teacher.md) — multiple per-user agents
- [Build: content pipeline](build-content-pipeline.md) — lore generation
