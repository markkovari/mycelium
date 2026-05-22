//! Shared wasm-component-loading substrate.
//!
//! Two consumers depend on this crate today:
//!   * `mycelium-tool-runner` — loads skill components (`mycelium:tool/tool-provider`)
//!   * `mycelium-hook-runner` — loads hook components (`mycelium:hook/hook-provider`)
//!
//! Both want the same loader, manifest schema, OCI/Nats fetchers, capability
//! allow-list, and per-call wasmtime config. Only the WIT export name and
//! request/response marshalling differ — those stay in each runner.

pub mod loader;
pub mod manifest;

pub use loader::{Loader, LoaderConfig};
pub use manifest::{ComponentManifest, ComponentSource};
