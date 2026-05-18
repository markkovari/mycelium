# User Journeys

Three audiences. End users chat with agents. Operators run them. Builders extend them. Each persona ships as one narrative `.md` + one `.feature` Gherkin file. Use the Gherkin as a smoke-test target; use the markdown to onboard a new dev / PM / operator to the persona.

## End users (talk to agents)

| # | Persona | One-liner | Doc | Feature |
|---|---------|-----------|-----|---------|
| 01 | First-time Telegram chatter | "I added the bot, what now?" | [end-first-chat.md](end-first-chat.md) | [end-first-chat.feature](end-first-chat.feature) |
| 02 | Recipe helper user | "What can I make with eggs and rice?" | [end-recipe.md](end-recipe.md) | [end-recipe.feature](end-recipe.feature) |
| 03 | Student | Homework tutor by subject | [end-student.md](end-student.md) | [end-student.feature](end-student.feature) |
| 04 | Customer support seeker | Escalates from bot to human | [end-customer-support.md](end-customer-support.md) | [end-customer-support.feature](end-customer-support.feature) |
| 05 | Family group member | Shared assistant in family Telegram | [end-family-group.md](end-family-group.md) | [end-family-group.feature](end-family-group.feature) |
| 06 | Voice-memo user | Sends audio, gets text reply | [end-voice-memo.md](end-voice-memo.md) | [end-voice-memo.feature](end-voice-memo.feature) |
| 07 | Photo sharer | Sends image, gets description / extraction | [end-photo.md](end-photo.md) | [end-photo.feature](end-photo.feature) |
| 08 | Daily journaler | Recurring reflection prompts | [end-journal.md](end-journal.md) | [end-journal.feature](end-journal.feature) |
| 09 | Privacy-sensitive user | Right to be forgotten | [end-privacy.md](end-privacy.md) | [end-privacy.feature](end-privacy.feature) |
| 10 | Multilingual user | Asks in Hungarian, replies in same | [end-multilingual.md](end-multilingual.md) | [end-multilingual.feature](end-multilingual.feature) |
| 11 | Power switcher | Multiple agents, switches mid-chat | [end-power-switch.md](end-power-switch.md) | [end-power-switch.feature](end-power-switch.feature) |
| 12 | Accessibility-first | Screen reader, voice-only | [end-accessibility.md](end-accessibility.md) | [end-accessibility.feature](end-accessibility.feature) |
| 13 | Batched-action user | "Process these 200 emails" in one ask | [end-batch.md](end-batch.md) | [end-batch.feature](end-batch.feature) |
| 14 | Scheduled-digest subscriber | "Send me a summary every Monday 8am" | [end-scheduled-digest.md](end-scheduled-digest.md) | [end-scheduled-digest.feature](end-scheduled-digest.feature) |

## Operators / admins (run a bot)

| # | Persona | One-liner | Doc | Feature |
|---|---------|-----------|-----|---------|
| 13 | Non-dev bot owner | Wizard-style first agent setup | [op-first-bot.md](op-first-bot.md) | [op-first-bot.feature](op-first-bot.feature) |
| 14 | SRE oncall | Alerts → Telegram → runbook | [op-sre-oncall.md](op-sre-oncall.md) | [op-sre-oncall.feature](op-sre-oncall.feature) |
| 15 | Support team lead | Tiered triage + handoff | [op-support-team.md](op-support-team.md) | [op-support-team.feature](op-support-team.feature) |
| 16 | Game master | Persistent NPCs per campaign | [op-game-master.md](op-game-master.md) | [op-game-master.feature](op-game-master.feature) |
| 17 | Teacher | Per-student tutors | [op-teacher.md](op-teacher.md) | [op-teacher.feature](op-teacher.feature) |
| 18 | Small business owner | FAQ + booking bot | [op-small-business.md](op-small-business.md) | [op-small-business.feature](op-small-business.feature) |
| 19 | Batch ops admin | Nightly fleet jobs across thousands of items | [op-batch-jobs.md](op-batch-jobs.md) | [op-batch-jobs.feature](op-batch-jobs.feature) |
| 20 | Cron scheduler admin | Recurring jobs (daily, weekly, cron expr) | [op-cron-scheduler.md](op-cron-scheduler.md) | [op-cron-scheduler.feature](op-cron-scheduler.feature) |

## Builders / developers (extend & ship)

| # | Persona | One-liner | Doc | Feature |
|---|---------|-----------|-----|---------|
| 19 | Solo developer | Local cycle + ghcr ship | [solo-developer.md](solo-developer.md) | [solo-developer.feature](solo-developer.feature) |
| 20 | SaaS founder | Embed REST in product | [build-saas-embed.md](build-saas-embed.md) | [build-saas-embed.feature](build-saas-embed.feature) |
| 21 | Edge Pi operator | Home automation on Raspberry Pi | [build-edge-pi.md](build-edge-pi.md) | [build-edge-pi.feature](build-edge-pi.feature) |
| 22 | Data analyst | A/B compare LLMs | [build-data-analyst.md](build-data-analyst.md) | [build-data-analyst.feature](build-data-analyst.feature) |
| 23 | Researcher | Long task + tool loop | [build-researcher.md](build-researcher.md) | [build-researcher.feature](build-researcher.feature) |
| 24 | Content pipeline | Editor + fact-check + translator | [build-content-pipeline.md](build-content-pipeline.md) | [build-content-pipeline.feature](build-content-pipeline.feature) |
| 25 | DevOps runbook author | Alert → diagnosis → fix proposal | [build-devops-runbook.md](build-devops-runbook.md) | [build-devops-runbook.feature](build-devops-runbook.feature) |
| 26 | Journalist tools | Transcribe + summarize + cite | [build-journalist.md](build-journalist.md) | [build-journalist.feature](build-journalist.feature) |
| 27 | Batch pipeline builder | Wire mycelium:batch in a fan-out worker | [build-batch-pipeline.md](build-batch-pipeline.md) | [build-batch-pipeline.feature](build-batch-pipeline.feature) |
| 28 | Cron scheduler builder | Wire mycelium:cron-style triggers via NATS | [build-cron-scheduler.md](build-cron-scheduler.md) | [build-cron-scheduler.feature](build-cron-scheduler.feature) |

## Cross-cutting capability map

| Capability | Personas |
|---|---|
| Telegram inbound | 01, 02, 03, 04, 05, 06, 07, 08, 10, 11, 13, 14, 16 |
| Telegram outbound (replies) | 01–11, 14, 16, 21 |
| REST API direct | 04, 12, 19, 20, 22, 23, 24, 25, 26 |
| Per-agent KV config | all operator + builder personas |
| Multiple agents per user | 11, 15, 16, 17, 24 |
| Voice / audio | 06, 12 |
| Image / multimodal | 07, 26 |
| Tool calls (web search, calc, etc.) | 02, 03, 14, 22, 23, 25, 26 |
| Long-running tasks | 04, 14, 17, 22, 23, 25 |
| Edge / offline | 09, 14, 18, 21 |
| Privacy / deletion | 09, 15 |
| Multi-language | 10, 24, 26 |
| Accessibility | 12 |

## Telegram ingress: long polling (default)

`mycelium-telegram-poll` runs `telegram-poller` as a Service component that issues `getUpdates` long-polls every ~2s. Outbound HTTP only. **No public URL needed.** Works behind NAT, on a Pi, on a laptop. Drop ngrok / Cloudflare Tunnel / custom domain from the requirements list.

Webhook variant (`mycelium-telegram-in` + `telegram-gateway`) was removed because v2 host's HTTP-exporter limitation blocked the bot from publishing to NATS, and it forced public-URL setup. Polling solves both at once. Historical commits retain the webhook code if needed.

## Conventions

- HTTP examples send `Host: localhost` to hit `mycelium-api`.
- Telegram chat events arrive on NATS subject `mycelium.channel.in` (published by the poller).
- `${BOT}` is the BotFather token of the operator's Telegram bot.
- `${CHAT}` is the user's `chat_id` in Telegram terms.
- KV bucket names bare (`mycelium-agent-config`, `mycelium-channel-sessions`, etc.).
- `${HOST_ID}` is the running wash host id (`cat /tmp/mycelium-v2-host-id`).
- "User says X" in Telegram = a webhook POST with `message.text = X`.

## Test-running the Gherkin

```bash
# pick any runner; the .feature files are vanilla Gherkin
cucumber docs/user-journeys/*.feature
# or behave (python)
behave docs/user-journeys/
# or godog (Go)
godog run docs/user-journeys/
```

For purely manual smoke-test, every `.md` ends with a "Connects to" link list — chain through three or four to validate cross-feature paths.
