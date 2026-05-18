# 03 — Student

**Persona**: Lila, 14. Math homework, then biology.
**Goal**: Hints, not answers. Show steps.
**Primitives**: One agent per subject. `mycelium-memory` for "show work" preference.

## Setup

Operator (parent/teacher) pre-creates two agents:
```bash
curl -X POST http://localhost:8080/agents -H 'host: localhost' \
  -d '{"id":"tutor-math","name":"Math tutor","system_prompt":"Never give the final answer. Walk through steps. Ask the student to fill the last step.","model":"qwen2.5:0.5b","tools":["calculator"],"max_steps":6}'

curl -X POST http://localhost:8080/agents -H 'host: localhost' \
  -d '{"id":"tutor-bio","name":"Bio tutor","system_prompt":"Use diagrams in text where useful. Cite the textbook.","model":"qwen2.5:0.5b","tools":["web-search"],"max_steps":6}'
```

## Happy path

```
Lila:  what is 7/12 + 5/8
Bot:   First find a common denominator. What's the LCM of 12 and 8?
Lila:  24
Bot:   Right. Now rewrite each fraction with denominator 24…
```

## Branches

### A. Lila tries to bait the answer
- "just tell me the answer". Agent's `system_prompt` resists. Replies: "I can show you, but you'll learn faster doing it. Want a hint?"

### B. Switch subject
- "/agent tutor-bio" → next message goes to bio agent. (Power-switcher pattern; see [end-power-switch](end-power-switch.md).)

### C. Calculator tool call
- Bot triggers `mycelium.tool.call` subject with `{tool_id:"calculator", args_json:"{\"expr\":\"7/12+5/8\"}"}`. tool-runner returns `35/24`. Agent uses internally but doesn't reveal until step is reached.

### D. Persistent progress
- Memory key `memory/lila/math/topic` = "fractions" → next session bot resumes.

### E. Parent dashboard
- Parent uses REST `GET /conversations?agent_id=tutor-math` to see Lila's session log (operator side, parental).

## Failure modes

| Symptom | Cause | Workaround |
|---|---|---|
| Bot just gives the answer | system_prompt drift | review + reset via PUT /agents/tutor-math |
| Calculator wrong | tool error | tool-runner log; switch to a different tool id |
| Bot inappropriate for age | model choice | swap to safer model in KV |

## Connects to

- [Teacher operator](op-teacher.md)
- [Power-switcher](end-power-switch.md)
- [Family group](end-family-group.md) — parent watches in shared chat
