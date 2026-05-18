# 13 — Non-dev bot owner

**Persona**: Klara. Yoga teacher. Wants a Telegram bot for class FAQ.
**Goal**: Set up first agent without reading code.
**Primitives**: A wizard CLI (or a Telegram admin chat) wraps the REST `POST /agents`.

## Path

```bash
# Klara runs the wizard
just wizard

# Wizard prompts:
> Bot name: yogabot
> Greeting: "Hi! I help with class times and signups."
> Telegram bot token: <paste>
> Telegram webhook secret: <auto-generate>
> Model: [1] qwen-fast  [2] gpt-4o-mini  [3] claude-haiku
> Pick: 2
> Tools to enable: [x] calendar [x] email-confirmation [ ] web-search
> Done. Test: send "hi" to your bot now.
```

Wizard does under the hood:
- `POST /agents` with chosen config
- writes `telegram.bot_token` and `telegram.webhook_secret` to wasi:config
- runs `setWebhook` against Telegram API pointing at `https://<host>/webhook`
- restarts `mycelium-telegram-in` to pick up config

## Branches

### A. Onboarding nudge
- 24h after wizard, if no Telegram traffic seen, bot DMs Klara with "need help?"

### B. Template gallery
- Wizard offers presets: "class FAQ", "personal coach", "small biz support", "family bot"
- Each preset is a YAML in `infra/agent-templates/` that the wizard expands

### C. Tool toggle without rebuild
- Wizard never recompiles WASM — only KV writes
- Adding a tool = enabling it in the agent's `tools` list in KV

### D. Branding
- Wizard accepts a logo upload (PNG) and a colour
- Saved to `memory/<agent>/brand` for web frontend

### E. Pause the bot
- `just bot-pause yogabot` → flip a flag in KV, agent replies with "bot is offline today"

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Wizard exits without saving | bad token | re-run; secrets cached short-term |
| Telegram setWebhook 401 | wrong token | wizard verifies token first |
| No replies after setup | webhook URL not public | wizard offers ngrok tunnel |
| First message ignored | telegram-gateway hadn't picked up new config | wizard restarts the workload |

## Connects to

- [End: first chat](end-first-chat.md) — what Klara's users see
- [Op: small business](op-small-business.md) — the deeper FAQ + booking flow
- [Build: SaaS embed](build-saas-embed.md) — same wizard, white-labelled
