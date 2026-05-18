# 06 — Voice memo user

**Persona**: Robi. Driving. Sends voice notes instead of typing.
**Goal**: Speak, get text reply (or audio reply if hands-free).
**Primitives**: Telegram voice → STT tool → agent step → reply (text + optional TTS audio).

## Path

Telegram delivers a `message.voice` (file_id, mime_type=ogg/opus).
telegram-gateway downloads via `getFile` → publishes raw audio to `mycelium.channel.audio.in` (future component).
A `transcriber` workload subscribes, calls Whisper-compatible HTTP endpoint, publishes text to `mycelium.task.submit`.

## Happy path

```
Robi sends voice note: "remind me to call mum tonight"
Bot text reply: "Got it. Reminder set: 'call mum' tonight 19:00."
```

## Branches

### A. Audio reply
- User toggle `/voice on` → bot replies via TTS as audio.
- TTS via separate workload that consumes `mycelium.step.result` for that conversation, calls TTS API, uploads file to Telegram.

### B. Long memo (3+ min)
- Chunked transcription with streaming results.
- Bot replies "still listening..." after 5s.

### C. Low-confidence transcription
- STT returns confidence < 0.6 → bot asks "I heard 'X' — correct?"

### D. Off-grid (Pi at home)
- All STT runs locally (whisper.cpp). No external calls.
- Useful when on a slow mobile connection too — the Pi takes the upload.

### E. Multiple voices in one memo
- Treated as one stream. Bot replies once, not per-speaker.
- (Future: diarisation feature).

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| No reply at all | transcriber component down | fallback "send as text?" message |
| Garbled text | wrong language hint | look up `memory/<user>/lang` |
| Hangs on big file | upload timeout | chunked download via Telegram API |

## Connects to

- [Multilingual](end-multilingual.md)
- [Accessibility](end-accessibility.md)
- [Edge Pi](build-edge-pi.md) — local STT
