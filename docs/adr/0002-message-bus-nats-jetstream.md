# ADR-0002: NATS JetStream as Sole Message Bus

**Status:** Accepted

## Context

Components need to communicate asynchronously. We need durable message delivery (tasks must survive restarts), pub/sub fan-out (router → multiple subscribers), request/reply (CLI ↔ session-bridge), and eventually scheduled/batch triggers. We also need this to work in local dev (single node) and production (clustered) without changing component code.

Options considered:
- Kafka + REST
- RabbitMQ
- Redis Streams
- NATS core + JetStream

## Decision

Use **NATS JetStream exclusively** for all inter-component messaging.

- Core NATS for request/reply and ephemeral pub/sub (pairing, CLI channel I/O).
- JetStream streams for durable, replayable event log (`MYCELIUM_TASKS`, `MYCELIUM_EVENTS`, `MYCELIUM_CONVERSATIONS`, etc.).
- JetStream KV buckets for mutable state (`mycelium-memory`, `mycelium-channel-sessions`, `mycelium-router-rules`).
- Subject namespace: `mycelium.<domain>.<action>[.<qualifier>]`.
- Stream and KV bucket init is idempotent (`just init-streams`).

## Consequences

- **+** Single infrastructure dependency for messaging, scheduling, and state.
- **+** Subject-based routing means adding a new consumer requires zero producer changes.
- **+** JetStream consumers enable exactly-once delivery and replay for audit/debug.
- **+** wasmCloud's `wasmcloud:messaging` capability maps directly to NATS subjects — no adapter layer needed.
- **-** NATS must be running for any component to function, including local dev.
- **-** No cross-cloud/broker federation without NATS Leaf Nodes or gateway configuration.
- **-** Debugging requires NATS CLI familiarity (`nats sub`, `nats stream info`).
