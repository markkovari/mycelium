# 22 — Data analyst (multi-model A/B)

**Persona**: Csilla. PM at a SaaS. Wants to know which LLM gives best draft quality.
**Goal**: Send the same 200 prompts to 4 models, score outputs, pick winner.
**Primitives**: Batch + per-item agent override + scoring agent + report.

## Architecture

```
input.jsonl (200 prompts)
   │
   └── POST /batches (one batch per model variant)
            │
            ├── batch-A: agent_id=draft-qwen
            ├── batch-B: agent_id=draft-llama
            ├── batch-C: agent_id=draft-gpt4o
            └── batch-D: agent_id=draft-claude
                          │
                          └── per-prompt step.result lands in KV

After all 4 batches done:
   scorer-agent (separate batch) reads outputs, scores each against rubric
   reporter-agent produces a markdown comparison table
```

## Setup

```bash
# Create 4 agent variants differing only by model + endpoint
for variant in qwen llama gpt4o claude; do
  curl -X POST http://localhost:8080/agents -H 'host: localhost' -d "{
    \"id\":\"draft-$variant\",
    \"name\":\"draft-$variant\",
    \"system_prompt\":\"Write a concise email reply\",
    \"model\":\"<model-$variant>\",
    \"tools\":[],\"max_steps\":1
  }"
done

# Update KV with per-variant endpoint + api_key
nats kv put mycelium-agent-config agent/draft-gpt4o '{"...","endpoint":"https://api.openai.com/v1/chat/completions","api_key":"sk-..."}'

# Submit 4 batches
for variant in qwen llama gpt4o claude; do
  curl -X POST http://localhost:8080/batches -H 'host: localhost' -d "{
    \"items_file\":\"prompts-200.jsonl\",
    \"agent_id\":\"draft-$variant\"
  }"
done
```

## Branches

### A. Cost tracking per variant
- Each step.result is enriched with `tokens_in`, `tokens_out`, `cost_usd`
- Reporter aggregates: cost per acceptable output

### B. Blind scoring
- Scorer doesn't know which variant produced each output
- Inputs to scorer shuffled by `mycelium.batch.<id>.shuffle` rule

### C. Human-in-the-loop scoring
- Subset (10%) goes to a human reviewer queue alongside model scoring
- Inter-rater reliability computed

### D. Stratified prompts
- Prompts tagged by category (formal, casual, complaint, sales)
- Report shows winner per category

### E. Continual learning
- Best variant per category fed back into agent KV
- Future drafts auto-route to category winner

### F. Drift detection
- Re-run the same 200 prompts monthly
- Alert when a variant's score drops > 10%

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Apples-to-oranges | system_prompts differ | enforce identical system_prompts across variants |
| One variant rate-limited | upstream 429s | budget-aware dispatcher |
| Score noise | model temperature high | fix temp to 0 in scoring agent |
| Cost surprise | flagship models on full 200 | dry-run estimate before submit |

## Connects to

- [Op: batch jobs](op-batch-jobs.md)
- [Build: content pipeline](build-content-pipeline.md)
- [Build: researcher](build-researcher.md) — same batch pattern
