# 23 — Researcher (long task + tool loop)

**Persona**: Dora. Academic. Wants a 30-minute "deep research" run.
**Goal**: Single ask → many tool calls → final report with citations.
**Primitives**: Agent loop with `max_steps: 30`, tool-runner, conversation history, cost cap.

## Architecture

```
Dora: "compare CRDT performance in 4 papers"
   │
   └── researcher-agent
         loop while max_steps not reached:
           plan next move
           tool call: web-search OR arxiv-fetch OR pdf-extract OR cite-format
           tool result merged into conversation
         finally: emit summary report with citations
```

## Setup

```bash
curl -X POST http://localhost:8080/agents -H 'host: localhost' -d '{
  "id":"researcher",
  "name":"Researcher",
  "system_prompt":"You research topics deeply. Plan tool use. Cite every claim. Stop when you have answers to all sub-questions.",
  "model":"claude-sonnet-latest",
  "tools":["web-search","arxiv-fetch","pdf-extract","cite-format","calculator"],
  "max_steps":30
}'
```

## Branches

### A. Budget cap
- `cost_usd_max` in agent config; agent aborts on overshoot
- Final reply includes "stopped due to budget" if reached

### B. Resumable
- Conversation persists across sessions
- Dora can return tomorrow: "continue from where we left"

### C. Live status
- Each tool call emits `mycelium.event.research.progress`
- A web dashboard streams them

### D. Stream of consciousness mode
- "/verbose" — agent prints its plan before each tool call
- "/silent" — only final report

### E. Cross-checking
- For each claim, agent runs 2 independent searches
- Disagreements flagged in the final report

### F. Hand-off to writer
- Final report dispatched to `writer-agent` for prose polish
- Two-agent chain

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Loop never converges | weak stopping criterion | enforce max_steps + budget |
| Citations made-up | model hallucination | cite-format tool verifies URLs |
| Tool failure cascades | no retry | per-tool retry policy |
| Final too long | unbounded summary | hard char limit on report |

## Connects to

- [Op: batch jobs](op-batch-jobs.md) — many researchers in parallel
- [Build: journalist](build-journalist.md) — same tool palette
- [End: power-switch](end-power-switch.md) — hand off to writer
