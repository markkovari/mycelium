# 26 — Journalist tools

**Persona**: Reka. Investigative journalist. Audio interviews + source verification.
**Goal**: Upload interview → transcript → fact-check → cited draft.
**Primitives**: Transcribe + entity-extract + source-check tool + writer chain.

## Architecture

```
upload audio.mp3 ──► /interviews
   │
   └── transcriber → publish mycelium.event.interview.transcribed
            │
            └── entity-extractor (names, places, dates)
                  publishes mycelium.event.interview.entities
                     │
                     └── source-checker (cross-references claims with public sources)
                           publishes mycelium.event.interview.checked
                              │
                              └── writer composes draft with footnoted citations
```

## Setup

```bash
curl -X POST http://localhost:8080/agents -H 'host: localhost' -d '{
  "id":"journalist-writer",
  "system_prompt":"Write in AP style. Cite every factual claim. Distinguish opinion from fact.",
  "model":"claude-sonnet-latest","tools":["cite-format","public-records"],"max_steps":8
}'
```

## Branches

### A. Off-the-record markers
- Interviewee says "off the record" at 12:33
- Entity-extractor flags everything until "back on record"
- Writer excludes those segments

### B. Multi-source corroboration
- Each entity (e.g. person name) gets cross-referenced
- Discrepancies between sources surface as a "verification needed" callout

### C. Speaker diarisation
- Interview with 3 voices → transcript shows speaker A/B/C
- Reka can rename A→"Mayor X" in dashboard

### D. Quote isolation
- Direct quotes get pulled into a quotes-bank for the writer
- Verified verbatim, no paraphrasing

### E. Sensitive claim flagging
- Claims about living people that are negative → require 2 independent sources before draft accepts

### F. Translation pipeline
- Source is Hungarian audio, article in English
- Transcript stays HU; writer outputs EN with HU quotes preserved + translations

### G. Legal hold
- Reka marks parts of conversation as "do not delete"
- Retention sweep skips those

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Misattributed quote | diarisation error | speaker re-naming UI |
| Public record link broken | source link rot | archived snapshot capture at fetch time |
| Mid-interview connection drop | partial transcript | resumable upload |
| Editorialised draft | weak system_prompt | strict opinion vs fact rubric |

## Connects to

- [Build: researcher](build-researcher.md)
- [Build: content pipeline](build-content-pipeline.md)
- [End: voice memo](end-voice-memo.md) — same audio path, shorter
