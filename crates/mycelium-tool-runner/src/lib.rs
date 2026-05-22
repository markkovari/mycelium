//! mycelium-tool-runner: native NATS dispatcher that embeds wasmtime to
//! execute untrusted skill components in a capability-sandbox.
//!
//! Subscribes to `mycelium.tool.call.>`, parses the tool name from the
//! subject, looks up the skill manifest in JetStream KV, fetches the wasm
//! bytes (OCI / NATS Object Store / disk, sha256-verified), runs it under
//! fuel + epoch interruption + memory cap, and publishes
//! `mycelium.tool.result` with the output.
//!
//! Standalone process; wasmtime is the only wasm runtime in mycelium.

pub mod config;
pub mod kv_host;
pub mod mcp_registry;
pub mod registry;
pub mod sandbox;

pub use mycelium_wasm_host::{
    loader::Loader, manifest::ComponentKind, manifest::ComponentManifest as SkillManifest,
    manifest::ComponentSource as SkillSource,
};
