//! mycelium-hook-runner: native NATS request-reply responder that loads
//! sandboxed hook components and dispatches them to mycelium-core's
//! lifecycle events.
//!
//! Mirror of mycelium-tool-runner: same `wasmtime` sandbox, same
//! capability allow-list, same OCI/NATS-object-store loader (via
//! `mycelium-wasm-host`). Differences:
//!   * WIT export name is `mycelium:hook/hook-provider.handle`, not
//!     `mycelium:tool/tool-provider.invoke`.
//!   * Hook input is `(event-name, payload-json)`; output is
//!     `result<string, string>`.
//!   * Subject is `mycelium.hook.<event-name>` (NATS request-reply); the
//!     responder publishes via `msg.respond(...)`.

pub mod config;
pub mod registry;
pub mod sandbox;

pub use mycelium_wasm_host::{
    loader::Loader, manifest::ComponentManifest as HookManifest,
    manifest::ComponentSource as HookSource,
};
