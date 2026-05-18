# 24 — Content pipeline builder

**Persona**: Levi. Marketing team lead. Wants editor → fact-checker → translator chain.
**Goal**: Submit one draft → get final polished + checked + translated copy in 4 languages.
**Primitives**: Multi-agent chain via NATS event flow, batch fan-out for translations.

## Architecture

```
POST /content/draft (draft.md)
   │
   └── editor-agent (style + grammar)
         publishes mycelium.event.content.edited
            │
            └── fact-checker-agent (verifies claims with web-search)
                  publishes mycelium.event.content.checked
                     │
                     └── translator-batch
                           ├── translator-en
                           ├── translator-de
                           ├── translator-hu
                           └── translator-es
                              all publish mycelium.event.content.translated.<lang>
                                 │
                                 └── on 4/4 → final-bundle event → Slack/email/Telegram
```

## Setup

```bash
# Chain-driver component watches content events and dispatches next stage
# Each stage is a separate agent in KV; pipeline definition in pipeline-config KV

nats kv put mycelium-pipelines pipeline/marketing-default '{
  "stages":["editor","fact-checker","translator"],
  "translator":{"languages":["en","de","hu","es"]}
}'
```

## Branches

### A. Per-stage approval
- After fact-checker, a human reviews before translation kicks off
- "/approve" / "/reject reason"

### B. Conditional skip
- Short tweets skip fact-checker (configurable threshold by content length)

### C. Domain-specific fact-checkers
- legal-content uses `legal-fact-checker` with stricter rubric

### D. Translation memory
- KV `memory/translations/<source-hash>/<lang>` caches prior translations
- Saves cost on repeat phrases

### E. Brand voice enforcement
- editor reads `memory/brand/voice` for tone, terms-to-avoid, glossary
- Updates flow without redeploy

### F. Quality scoring
- After translation, a scorer agent rates each lang version
- Below threshold → routes back to translator with feedback

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Stage stalls | next-stage subscriber down | health check + alert |
| Translation drift | model swap | regression test corpus |
| Loop on rejected | bad reject reason | finite retry, hard stop after 2 |
| Cost runaway | flagship at every stage | tier model by stage |

## Connects to

- [Build: data analyst](build-data-analyst.md) — same fan-out idea
- [End: multilingual](end-multilingual.md)
- [Op: batch jobs](op-batch-jobs.md) — translator parallelism
