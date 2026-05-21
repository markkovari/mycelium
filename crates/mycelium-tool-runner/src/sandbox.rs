//! wasmtime sandbox host for skills.
//!
//! Per-invocation lifecycle:
//! 1. Validate manifest capabilities against the operator allow-list.
//! 2. Build a fresh wasmtime::Store with fuel, an epoch deadline and a
//!    memory cap derived from the manifest (or runner defaults).
//! 3. Construct a Linker that only adds host imports the manifest declared,
//!    so a `calc` skill physically cannot reach wasi:http.
//! 4. Instantiate the component, call `tool-provider.invoke`, return the
//!    result JSON.
//!
//! Phase 3.2 will fill in the actual `wasmtime::component::Linker` wiring
//! against the slim `mycelium:skill/skill` WIT world. For now the surface
//! exists, the deps compile, and `invoke` returns a structured "not yet
//! implemented" error so the rest of the pipeline can be exercised.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use crate::{config::Config, manifest::SkillManifest};

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCallReq {
    pub call_id: String,
    /// Tool/skill name. Routed via the NATS subject by the dispatcher; carried
    /// here only for convenience.
    #[serde(default)]
    pub tool_id: String,
    /// Raw JSON string as Gemini emits it. The skill parses it itself.
    pub arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCallResp {
    pub call_id: String,
    #[serde(default)]
    pub tool_id: String,
    pub output_json: String,
    pub is_error: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum SandboxError {
    #[error("skill {0} requested capability {1} which is not in the operator allow-list")]
    CapabilityRefused(String, String),
    #[error("skill {0} exceeded wall-clock deadline ({1} ms)")]
    Timeout(String, u64),
    #[error("skill {0} ran out of fuel")]
    OutOfFuel(String),
    #[error("skill {0} hit memory cap ({1} bytes)")]
    OutOfMemory(String, u64),
    #[error("skill {0} trapped: {1}")]
    Trap(String, String),
    #[error("skill loader failed: {0}")]
    Loader(#[from] anyhow::Error),
    #[error("sandbox internal error: {0}")]
    Internal(String),
}

#[derive(Clone)]
pub struct Sandbox {
    config: Arc<Config>,
    // wasmtime::Engine is internally Arc — kept here for future use when
    // Phase 3.2 wires up the actual instantiation.
    #[allow(dead_code)]
    engine: wasmtime::Engine,
}

impl Sandbox {
    pub fn new(config: Arc<Config>) -> Result<Self> {
        let mut wc = wasmtime::Config::new();
        wc.async_support(true);
        wc.consume_fuel(true);
        wc.epoch_interruption(true);
        // Allow component-model.
        wc.wasm_component_model(true);
        let engine = wasmtime::Engine::new(&wc)?;
        Ok(Self { config, engine })
    }

    pub async fn invoke(
        &self,
        manifest: &SkillManifest,
        _wasm_bytes: &[u8],
        req: ToolCallReq,
    ) -> std::result::Result<ToolCallResp, SandboxError> {
        // Capability gate.
        for cap in &manifest.capabilities {
            if !self.config.capability_approved(cap) {
                return Err(SandboxError::CapabilityRefused(
                    manifest.name.clone(),
                    cap.clone(),
                ));
            }
        }

        // Resource caps come from manifest, fall back to config defaults.
        let _fuel = manifest
            .fuel_limit
            .unwrap_or(self.config.default_fuel_limit);
        let _mem_max = manifest
            .memory_max_bytes
            .unwrap_or(self.config.default_memory_max);
        let wall_deadline_ms = manifest
            .wall_deadline_ms
            .unwrap_or(self.config.default_wall_deadline_ms);
        let _deadline = Duration::from_millis(wall_deadline_ms);

        // TODO(Phase 3.2): build wasmtime::component::Linker against
        // `mycelium:skill/skill` WIT world; add only the imports declared in
        // `manifest.capabilities`; instantiate the component; call
        // `tool-provider.invoke(req)` under fuel + epoch interruption.
        Ok(ToolCallResp {
            call_id: req.call_id,
            tool_id: manifest.name.clone(),
            output_json: r#"{"error":"sandbox not implemented yet (Phase 3.2)"}"#.into(),
            is_error: true,
        })
    }
}
