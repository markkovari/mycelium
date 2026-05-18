# 02 — Recipe helper user

**Persona**: Janos. Bored after work. Has eggs, rice, kimchi.
**Goal**: Get a recipe he can actually make in 20 minutes.
**Primitives**: Telegram → recipe agent with `tools: [web-search]` for citations.

## Happy path

```
Janos:    eggs rice kimchi 20 min
Bot:      Kimchi fried rice. Want the recipe?
Janos:    yes
Bot:      [4 steps, timings, swap suggestions]
          Source: ...
```

Behind: agent's `tools` list includes `web-search`. tool-runner dispatches the search call, agent stitches the result into its reply.

## Branches

### A. Allergy declared in profile
- Janos previously said "no peanuts" → stored in `mycelium-memory` bucket key `memory/janos/diet`.
- Agent reads memory at step time, filters suggestions.

### B. "I don't have rice"
- Mid-conversation substitution. Conversation history (`mycelium:conversation/conversations::get_messages`) gives context — bot doesn't ask twice.

### C. Voice memo with ingredients
- Janos sends an audio note listing 6 items.
- Routes through voice path ([end-voice-memo](end-voice-memo.md)).

### D. Saved to grocery list
- "/save" → writes structured shopping items to `memory/janos/shopping`.
- Later Janos `/list shopping` → bot reads memory, formats.

### E. Photo of pantry
- Image goes through [end-photo](end-photo.md) path.

## Failure modes

| Symptom | Cause | Workaround |
|---|---|---|
| Recipe references missing ingredient | tool search returned wrong source | "different one please" |
| Same recipe every time | model temperature too low | operator bumps in agent KV |
| Citations missing | web-search tool errored | tool-runner logs in mycelium-events-journal |

## Connects to

- [Daily journal](end-journal.md) — same memory bucket pattern
- [Voice memo](end-voice-memo.md)
- [Photo](end-photo.md)
- [Build: researcher](build-researcher.md) — tool-runner under the hood
