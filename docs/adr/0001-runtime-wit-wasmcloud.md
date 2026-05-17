# ADR-0001: WIT Component Model + wasmCloud as Runtime

**Status:** Accepted

## Context

mycelium needs to run heterogeneous workloads (HTTP ingress, LLM calls, NATS pub/sub, key-value storage, Telegram bot) across potentially many hosts. Each workload has different capability requirements and should be replaceable without rebuilding the whole system. We need strong interface contracts between components so that implementations can be swapped (e.g., switch LLM provider, replace KV backend) without touching callers.

Options considered:
- Monolithic Rust binary with feature flags
- gRPC microservices
- WIT-based components on wasmCloud

## Decision

Use the **WebAssembly Component Model (WIT)** for interface contracts and **wasmCloud** as the distributed component runtime.

- Every capability is expressed as a WIT interface (`wit/*.wit`).
- Components are compiled to `wasm32-wasi` `cdylib` targets.
- wasmCloud hosts run components and wire capability providers (NATS, HTTP, KV) at deploy time via WADM manifests.
- Native Rust crates (`crates/`) exist only for the CLI and integration tests — they cannot run inside wasmCloud.

## Consequences

- **+** Interface contracts are enforced at compile time via WIT bindings; callers break at build time if an interface changes.
- **+** Components are capability-safe: a component cannot open a TCP socket or read the filesystem unless explicitly linked to a provider.
- **+** Hot-swap: update a component OCI image without restarting the host or other components.
- **-** `cargo check` on WASM components requires seeded `wit/deps/` (run `just init-wit-deps` after clone).
- **-** WASM components cannot use arbitrary crates from crates.io — only those compatible with `wasm32-wasi`.
- **-** Local dev loop is heavier than a plain binary: requires `wash`, `wkg`, and a running wasmCloud host.
