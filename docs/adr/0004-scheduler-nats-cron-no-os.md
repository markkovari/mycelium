# ADR-0004: NATS-Based Distributed Scheduler (No OS Cron)

**Status:** Accepted

## Context

Users need to trigger agent actions on a schedule (e.g., "summarise my emails every morning", "check stock prices every hour"). Options for implementing this:

1. OS-level `cron` — host-specific, not portable to wasmCloud, breaks multi-host deployments.
2. wasmCloud built-in timer capability — ties scheduling to a specific host process.
3. External scheduler service (Temporal, Airflow) — heavy dependency.
4. NATS JetStream scheduled publish — NATS natively supports publishing a message to a subject at a cron schedule via `nats-server` configuration or a thin scheduler component.

## Decision

Implement scheduling entirely over **NATS JetStream** with a dedicated `cron` WIT interface. No OS-level cron or external scheduler.

### WIT interface (`wit/cron.wit`)

```wit
package mycelium:cron@0.1.0;

interface scheduler {
    record cron-job {
        id:       string,
        schedule: string,   // standard 5-field cron expression
        subject:  string,   // NATS subject to publish trigger to
        payload:  string,   // JSON payload attached to trigger message
    }

    register:   func(job: cron-job) -> result<string, string>;
    unregister: func(id: string)    -> result<_, string>;
    list:       func()              -> list<cron-job>;
}
```

### Runtime behaviour

- A `cron-scheduler` component subscribes to `mycelium.cron.register` / `mycelium.cron.unregister` and maintains job state in the `mycelium-cron-jobs` JetStream KV bucket.
- On each tick, the scheduler component publishes the job's configured `subject` with the job's `payload`. Consumers of that subject do not know they were triggered by a cron job — the message looks identical to any other `ChannelMessage` or task submission.
- The scheduler itself is globally unique within a NATS cluster (single-active consumer pattern via JetStream). Any host can run the component; only one instance is active at a time.
- No OS scheduler, no `crontab`, no host-level configuration. Deploy the component; scheduling is live.

### Subject convention

```
mycelium.cron.register      # register a new job
mycelium.cron.unregister    # remove a job by id
mycelium.cron.tick.<id>     # internal heartbeat (scheduler → itself)
mycelium.cron.fire.<id>     # published when a job fires (auditable)
```

## Consequences

- **+** No host dependency — scheduler moves with the component when the host restarts or migrates.
- **+** Cron jobs survive NATS restarts (state in JetStream KV).
- **+** Single-active consumer guarantees exactly one scheduler fires per tick across a cluster.
- **+** Triggered subjects are indistinguishable from user-initiated messages — existing executor and agent components handle them unchanged.
- **-** Sub-second precision is not guaranteed; NATS publish latency adds jitter. Not suitable for high-frequency or latency-sensitive scheduling.
- **-** The scheduler component must be deployed for cron to function; it is not implicit in the runtime.
