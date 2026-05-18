# 12 — Accessibility-first user

**Persona**: Jakub. Blind. Uses VoiceOver on iPhone. Telegram audio replies preferred.
**Goal**: Fully usable without sight.
**Primitives**: Voice in + voice out, screen-reader-friendly text, alt-text on every link.

## Happy path

Jakub asks via voice. Bot replies via audio file. No images sent. No emoji clutter.

## Settings

```bash
nats kv put mycelium-agent-config agent/a11y '{
  "id":"a11y","name":"AccessBot",
  "system_prompt":"Reply in plain prose. No emoji. No tables. Spell out abbreviations. Provide alt-text for any link.",
  "model":"qwen2.5:0.5b","tools":["tts","stt"],"max_steps":4
}'
nats kv put mycelium-agent-config agent/a11y/voice_reply '{"on": true}'
```

## Branches

### A. Voice-first mode
- All replies as audio
- Text version included for skim re-reading

### B. Screen-reader-friendly text
- No `*bold*` or `_italic_` markdown
- Lists use plain prose: "First, ... Second, ..."
- Tables flattened to sentences

### C. Slow speech mode
- TTS rate 0.85x
- Stored in `memory/<user>/tts_rate`

### D. Skip emoji + verbose alt-text
- No emoji at all in replies
- Every URL is accompanied by a human-readable description: "link to the recipe: ..."

### E. Voice command vocabulary
- "Stop reading", "repeat last", "louder", "slower"
- Bot intercepts before agent step

### F. High-contrast / large-text mode
- For users with low vision (not blind)
- Replies use `<strong>` HTML hints if frontend renders them; Telegram ignores but a web frontend honours

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Audio reply garbled | TTS service down | text fallback with apology |
| Emoji slip in reply | system_prompt drift | post-process filter strips emoji |
| Markdown in reply | agent rendering | post-process to plain text |
| Long reply hard to follow | no chapters | bot inserts spoken "pause" markers |

## Connects to

- [Voice memo](end-voice-memo.md) — input side
- [Privacy](end-privacy.md) — voice transcripts opt-in
- [Multilingual](end-multilingual.md) — voice in user's language
