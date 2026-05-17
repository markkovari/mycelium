# ADR-0005: Batch AI — Scheduled Accumulation for Cheap Inference

**Status:** Accepted

## Context

Several use cases (nightly summaries, bulk document processing, background analysis) do not require real-time LLM responses. Batch AI providers (OpenAI Batch API, Anthropic Batch) offer ~50% cost reduction for asynchronous inference with up to 24-hour turnaround. These are fundamentally different from conversational calls:

- Input is accumulated over time, not sent one message at a time.
- The request is submitted as a file/batch to the provider, not a streaming HTTP call.
- Results are polled or delivered asynchronously hours later.
- Multiple user tasks can be bundled into a single provider batch to maximise discount.

## Decision

Add a **batch job** interaction mode alongside conversational (ADR-0003) and scheduled (ADR-0004). Batch jobs are scheduled AND accumulated before submission.

### WIT interface (`wit/batch.wit`)

```wit
package mycelium:batch@0.1.0;

interface batch-queue {
    record batch-job {
        id:          string,
        agent_id:    string,
        prompt:      string,
        context:     option<string>,   // serialised conversation context
        callback:    string,           // NATS subject to publish result to
        created_at:  string,           // ISO-8601
    }

    enqueue: func(job: batch-job)        -> result<string, string>;
    cancel:  func(id: string)            -> result<_, string>;
    status:  func(id: string)            -> result<string, string>; // "queued"|"submitted"|"done"|"error"
}
```

### Lifecycle

```
User / cron trigger
  │  enqueue(batch-job)
  ▼
mycelium.batch.enqueue          → batch-queue component stores job in
                                  mycelium-batch-pending KV bucket

[scheduled flush — configurable, e.g. every 6h or at midnight]
  │
batch-scheduler component
  │  reads all pending jobs
  │  groups by provider (openai / anthropic)
  │  submits batch file to provider API
  │  records batch_id → job_ids mapping in mycelium-batch-submitted KV
  ▼
mycelium.batch.submitted.<batch_id>   (audit event)

[poll loop — every 15–30 min]
batch-poller component checks provider for completed batches
  │  on completion: parse results
  │  publish each result to job.callback subject
  │  mark job done in KV
  ▼
mycelium.batch.result.<job_id>        → consumer (user notify, downstream task)
```

### Provider abstraction

The `batch-scheduler` component calls the provider via `wasi:http`. Provider selection is per-job (field in `batch-job` or derived from agent config). OpenAI and Anthropic batch formats differ; the component handles serialisation per provider.

### Interaction with cron (ADR-0004)

The flush trigger is itself a cron job registered via ADR-0004's scheduler. The batch system does not implement its own timer.

## Consequences

- **+** ~50% cost reduction on non-interactive workloads.
- **+** Accumulation decouples producer rate from provider submission rate; burst user activity does not cause burst API spend.
- **+** Results delivered over NATS — downstream components (notifications, follow-up tasks) consume them identically to real-time results.
- **+** Flush schedule is configurable per deployment (aggressive = lower latency, conservative = larger batches = cheaper).
- **-** Results can arrive up to 24h after enqueueing; not suitable for any user-facing synchronous flow.
- **-** Provider batch APIs have their own limits (max items per batch, max tokens). The batch-scheduler must chunk jobs to stay within limits.
- **-** Failure modes are async: a batch rejection from the provider surfaces hours after submission. Dead-letter handling on `mycelium-batch-failed` KV is required.
