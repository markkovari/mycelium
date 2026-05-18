# 07 — Photo sharer

**Persona**: Reka. Sends a photo of a wine label, asks "is this any good?"
**Goal**: Bot describes / extracts / answers based on image content.
**Primitives**: Telegram photo → image-handler workload → multimodal agent.

## Path

Telegram `message.photo` (array of sizes, biggest is last) → telegram-gateway downloads via `getFile` → publishes to `mycelium.channel.image.in` with `chat_id, file_url, caption`.
An `image-handler` workload subscribes, calls a multimodal LLM endpoint (Anthropic, GPT-4o, LLaVA local), publishes to `mycelium.task.submit` as a task with `input` set to `[CAPTION]+[image-url-or-base64]`.

## Happy path

```
Reka: [photo of label] "is this good?"
Bot:  "That's a 2018 Furmint from Tokaj — well-reviewed. Pairs with: roast pork, aged cheese."
```

## Branches

### A. OCR-only mode
- "/ocr" caption → just extract text, no commentary.
- Useful for receipts, business cards.

### B. Album of photos
- Multiple photos in one message → batch-style: one task per photo, single coalesced reply.
- See [Batch](end-batch.md) primitive.

### C. Pantry inventory
- "/pantry" caption → image-handler extracts items, writes to `memory/<user>/pantry`.
- Connects to [Recipe](end-recipe.md) flow.

### D. Plant ID
- Specialised `plant-id` agent that calls a plant-recognition tool.
- Operator selects agent: "send as @plantbot".

### E. Privacy-sensitive image
- User says "don't store this".
- Image-handler runs the inference but does NOT write the photo to `mycelium-images` KV. Audit log notes the opt-out.

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| No reply | image-handler down | retry queue + alert |
| Generic reply | model can't see image | check multimodal model id |
| Slow | large file | downscale before LLM call |
| OCR garbage | low-res / rotated | rotate + upscale tool |

## Connects to

- [Recipe](end-recipe.md) (pantry photo)
- [Journalist](build-journalist.md) (whiteboard photo)
- [Privacy](end-privacy.md)
