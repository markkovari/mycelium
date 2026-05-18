# 09 — Privacy-sensitive user

**Persona**: Sara. EU resident. Wants full control.
**Goal**: See what's stored. Export. Delete. Understand retention.
**Primitives**: REST `/me` endpoints, KV scan + purge, audit trail.

## Endpoints (proposed)

```bash
GET    /me/data?chat_id=...     # everything mycelium has for me
POST   /me/export                # async PDF/JSON export, mailed
DELETE /me/data?chat_id=...      # purge + tombstone
GET    /me/retention              # policy explanation
POST   /me/consent                # opt-in/out per data class
```

## Happy path

```
Sara → bot: "/privacy"
Bot:  "Here's what I have on you:
        - conversations: 14
        - messages: 287
        - memory entries: 9
        - journal entries: 0
       /privacy export | /privacy delete | /privacy policy"

Sara → bot: "/privacy export"
Bot:  "Compiling. You'll get a ZIP link within 15 minutes."
```

## Branches

### A. Per-data-class deletion
- "/privacy delete journal" — only the journal bucket entries
- "/privacy delete conversations" — only chat history
- Tombstones written to `mycelium-privacy-audit` KV

### B. Right to be forgotten (hard delete)
- "/privacy forget-me" — full purge across all KV buckets touching this chat_id / agent association
- Audit-only stub remains: `{chat_id, deleted_at, reason}`

### C. Consent dashboard
- "/privacy consent" → toggles per class (messages, voice transcripts, image bytes, telemetry)
- Components honour these flags at write time

### D. Local-only mode
- User opts into "all data stays on this Pi"
- Operator-side flag prevents OCI image pulls that hit external endpoints
- LLM endpoint must also be local (Ollama on the Pi)

### E. Subject access request (SAR)
- Operator gets a court order. Uses `GET /admin/data?chat_id=...&reason=...` (audit-logged).

### F. Retention sweep
- Cron job at 03:00 daily: `mycelium-retention-policy` KV has TTLs per bucket
- Items older than TTL are purged, tombstones logged

## Failure modes

| Symptom | Cause | Mitigation |
|---|---|---|
| Export fails | too much data | chunked job, per-day shards |
| Delete leaves traces | dangling references in other buckets | cross-bucket sweep checklist |
| User claims data deleted but it isn't | replication lag | confirm purge only after all replicas ack |
| Operator data exfil | insider risk | audit logs immutable (append-only stream) |

## Connects to

- [Edge Pi](build-edge-pi.md) — full local-only deployment
- [Customer support](end-customer-support.md) — deletion requests via support
- [Op: support team](op-support-team.md) — handling SAR
