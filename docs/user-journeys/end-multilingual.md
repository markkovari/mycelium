# 10 — Multilingual user

**Persona**: Levente. Writes in Hungarian, Pal in German, the kid in English.
**Goal**: Bot replies in the user's language, every time.
**Primitives**: Per-user `memory/<user>/lang` + multilingual-capable model.

## Happy path

```
Levente:  "mit főzzek vacsorára?"
Bot:      "Van otthon tojás és rizs? Ezekből 15 perc alatt készíthetsz kimcsis tojásrántottát."

Pal:      "wie wird das Wetter morgen?"
Bot:      "Morgen 18°C, sonnig — gute Joggingbedingungen."
```

## Branches

### A. Auto-detect first message
- First user message → run a lightweight language-id tool
- Save to `memory/<chat>/lang`
- All subsequent replies default to that

### B. Explicit override
- "/lang en" → switch immediately
- "/lang auto" → re-detect per message

### C. Mixed input (codeswitching)
- User mixes Hungarian and English mid-sentence
- Agent's system_prompt includes "match user's dominant language in current turn"

### D. Translation between members
- Family group: Pal types in German, Anna replies in Hungarian
- "/translate-to anna" — bot acts as bridge

### E. Slang and dialect
- "ezt nem tudom kérlek hagyjál békén" — informal HU
- Bot matches register: informal in informal language

### F. Locale-aware formatting
- Dates: "2026-05-18" vs "18.05.2026" vs "May 18, 2026"
- Numbers: comma vs period decimals

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Bot replies in wrong language | weak model | swap to multilingual flagship |
| Bot stuck on one language | lang preference never set | first-message auto-detect |
| Translation drifts meaning | low-temp + glossary needed | per-domain glossary in KV |

## Connects to

- [Family group](end-family-group.md) — bridging members
- [Content pipeline](build-content-pipeline.md) — translator agents
- [Journalist](build-journalist.md) — foreign-language sources
