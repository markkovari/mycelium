# 25 — DevOps runbook author

**Persona**: Jonas. Wants alerts → automated triage → proposed fix → optional auto-apply with guardrails.
**Goal**: Encode runbooks in versioned YAML; bot follows them; safe actions auto-applied; risky ones queued for human.
**Primitives**: Runbook DSL in KV + diagnostic tools + action-runner with permission tiers.

## Runbook DSL

```yaml
# memory/runbook/api-prod
name: api-prod
on:
  - alert: latency_p99_gt_threshold
steps:
  - tool: kube_pods_status
  - tool: deployment_recent_changes
  - decision: |
      if recent_change && rollback_safe: action=rollback
      elif pod_oom: action=scale_memory
      else: action=human
actions:
  rollback:
    permission: auto
    command: kubectl rollout undo deploy/api-prod
  scale_memory:
    permission: human-approve
    command: kubectl patch deploy/api-prod -p '{"spec":{"template":{"spec":{"containers":[{"name":"app","resources":{"limits":{"memory":"2Gi"}}}]}}}}'
  human:
    permission: human
    message: "Page Marek, novel pattern."
```

## Branches

### A. Permission tiers
- `auto` actions run with no human gate
- `human-approve` actions paused for Telegram "/approve"
- `human` actions never auto

### B. Dry-run mode
- `/runbook dry api-prod` runs the steps but doesn't execute actions
- Outputs what WOULD happen

### C. Rollback safety check
- Before `auto: rollback`, check that the previous version was healthy
- Refuse if no known-good version available

### D. Runbook versioning
- Each runbook commits to a git repo via outbound webhook
- KV value is `last_commit_sha` for audit

### E. Cross-service runbooks
- Linked runbooks: if api-prod fails, check db-prod too
- `requires:` field in YAML

### F. Postmortem auto-generation
- After resolution, runbook emits a `postmortem.draft` with the timeline of actions and outcomes

### G. Approval window
- `auto` actions only run during business hours
- Outside, downgrade to `human-approve`

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Wrong action auto-applied | misclassified alert | guardrail on per-action canary |
| Permission bypass | misconfigured runbook | linter rejects YAML without permission |
| Runbook drift | manual KV edits | git-sync nightly |
| Tool failure halts whole run | no fallback | each step has retry + skip-on-fail option |

## Connects to

- [Op: SRE oncall](op-sre-oncall.md) — alert ingestion
- [Op: cron scheduler](op-cron-scheduler.md) — business-hours gate
- [Build: researcher](build-researcher.md) — diagnostic search
