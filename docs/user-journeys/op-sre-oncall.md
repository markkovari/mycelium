# 14 — SRE oncall

**Persona**: Marek. Pager rotates weekly. Telegram is the alert surface.
**Goal**: Alert lands → bot triages → escalate to human if past first-pass thresholds.
**Primitives**: Inbound webhook from alerting → router → oncall agent → cron-driven follow-ups → Telegram.

## Path

```
Datadog / Grafana alert hits POST /alerts (host: localhost)
   ↓
alert-ingest workload normalises to event
   ↓
publish mycelium.event.alert.<severity>.<service>
   ↓
router rule: severity=high → publish mycelium.task.submit for "oncall-agent"
   ↓
oncall-agent (read runbook from KV, gather metrics tool calls)
   ↓
telegram-out → Marek's chat
   ↓
if Marek doesn't ack in 5 min → cron-scheduler escalates to backup oncall
```

## Happy path

```
22:14  Bot:  "🚨 api-prod p99 latency 2.1s (threshold 800ms).
              Likely: deployment 12 min ago. Suggested first step: rollback.
              /ack | /escalate | /silence 30m"

Marek: /ack
Bot:   "Ack'd. I'll stop paging on this alert."
```

## Branches

### A. Pre-baked runbooks per service
- `memory/runbook/<service>` holds steps
- Agent uses them in its prompt

### B. Self-healing suggestions
- Agent proposes a fix; Marek says "do it"; agent triggers `mycelium.action.execute` with bounded permissions

### C. Multi-step diagnostics
- "/diag api-prod" → triggers a research-agent batch over metrics, logs, traces
- Single coalesced summary

### D. Silence with reason
- `/silence 30m reason="known bug, fix in PR-1234"`
- Auto-resumes alerts after 30m

### E. Postmortem starter
- After incident resolved, `/postmortem` → bot pulls timeline from `mycelium-events-journal` and drafts a doc

### F. On-call handoff
- `/handoff @next-oncall` → Marek's pager rules pause, next oncall takes over
- Stored in `memory/oncall/current`

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Pages stack up | bot down | independent dead-man-switch outside mycelium |
| Wrong runbook | service mis-tag | strict service-name allowlist in `memory/runbook/*` |
| Alert flapping | thresholds too tight | bot proposes threshold change, op approves |

## Connects to

- [Build: devops runbook](build-devops-runbook.md)
- [Op: cron scheduler](op-cron-scheduler.md) — escalation timer
- [End: scheduled digest](end-scheduled-digest.md) — weekly oncall summary
