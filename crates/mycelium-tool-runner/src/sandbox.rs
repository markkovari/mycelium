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

use anyhow::{anyhow, Context as _, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use wasmtime::component::{Component, Linker, Val};
use wasmtime::{AsContextMut, Engine, Store, StoreLimitsBuilder};
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use crate::{config::Config, SkillManifest};

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

/// Per-invocation store state. Holds the WASI context (only populated if
/// the skill's manifest asked for `wasi:*` caps and the operator approved
/// them) plus the resource table wasmtime needs.
pub struct SkillCtx {
    pub wasi: Option<wasmtime_wasi::WasiCtx>,
    pub http: WasiHttpCtx,
    pub table: ResourceTable,
    pub limits: wasmtime::StoreLimits,
}

impl wasmtime_wasi::WasiView for SkillCtx {
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx {
        // SAFETY-ish: we only ever call WasiView::ctx() when at least one
        // wasi:* import is in the Linker, which only happens when self.wasi
        // is Some. Panic is the correct failure mode if that invariant
        // somehow breaks at runtime.
        self.wasi
            .as_mut()
            .expect("WasiView::ctx called but no WasiCtx was installed for this skill")
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiHttpView for SkillCtx {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

#[derive(Clone)]
pub struct Sandbox {
    config: Arc<Config>,
    engine: Engine,
}

impl Sandbox {
    pub fn new(config: Arc<Config>) -> Result<Self> {
        let mut wc = wasmtime::Config::new();
        wc.async_support(true);
        wc.consume_fuel(true);
        wc.epoch_interruption(true);
        wc.wasm_component_model(true);
        let engine = Engine::new(&wc)?;

        // Background ticker for epoch_interruption. Increment the engine's
        // epoch counter once per millisecond; any Store whose deadline is
        // expressed in epoch ticks will see its deadline hit at that
        // resolution. The ticker lives for the whole process — there is no
        // path to remove an epoch.
        let engine_for_tick = engine.clone();
        std::thread::Builder::new()
            .name("wasmtime-epoch".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(1));
                engine_for_tick.increment_epoch();
            })
            .context("spawn wasmtime epoch ticker thread")?;

        Ok(Self { config, engine })
    }

    pub async fn invoke(
        &self,
        manifest: &SkillManifest,
        wasm_bytes: &[u8],
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

        let fuel = manifest
            .fuel_limit
            .unwrap_or(self.config.default_fuel_limit);
        let mem_max = manifest
            .memory_max_bytes
            .unwrap_or(self.config.default_memory_max);
        let wall_deadline_ms = manifest
            .wall_deadline_ms
            .unwrap_or(self.config.default_wall_deadline_ms);

        let component = Component::from_binary(&self.engine, wasm_bytes)
            .map_err(|e| SandboxError::Internal(format!("load component: {e}")))?;

        let mut linker: Linker<SkillCtx> = Linker::new(&self.engine);
        // wasm32-wasip2 components always import wasi:io / wasi:cli /
        // wasi:filesystem transitively via wit-bindgen's runtime helpers,
        // even when the skill itself never calls wasi. Add the WASI host
        // unconditionally; the capability-gate is enforced at the manifest
        // layer (the WasiCtxBuilder below is the empty default — no FS, no
        // network, no env access). add_to_linker_async pulls every wasi:*
        // interface in, but the resources they expose are scoped to the
        // builder configuration.
        wasmtime_wasi::add_to_linker_async(&mut linker)
            .map_err(|e| SandboxError::Internal(format!("link wasi: {e}")))?;
        if manifest
            .capabilities
            .iter()
            .any(|c| c.starts_with("wasi:http"))
        {
            wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)
                .map_err(|e| SandboxError::Internal(format!("link wasi:http: {e}")))?;
        }

        // Minimal WasiCtx by default. Skills that declared wasi:* caps in
        // their manifest still only get whatever the builder is configured
        // for; we keep stderr inherit on so error output reaches journald.
        let wasi_ctx = Some(WasiCtxBuilder::new().inherit_stderr().build());
        let limits = StoreLimitsBuilder::new()
            .memory_size(mem_max as usize)
            .build();

        let mut store = Store::new(
            &self.engine,
            SkillCtx {
                wasi: wasi_ctx,
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
                limits,
            },
        );
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(fuel)
            .map_err(|e| SandboxError::Internal(format!("set fuel: {e}")))?;
        // Epoch tick is 1ms; deadline = wall_deadline_ms ticks from now.
        store.set_epoch_deadline(wall_deadline_ms.max(1));

        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .map_err(|e| classify_instantiate_err(&manifest.name, e))?;

        // The skill exports `mycelium:tool/tool-provider`. Look up the
        // exported interface by name on the instance, then the `invoke`
        // function inside it.
        let iface_idx = instance
            .get_export(store.as_context_mut(), None, "mycelium:tool/tool-provider@0.1.0")
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "skill {} does not export mycelium:tool/tool-provider@0.1.0",
                    manifest.name
                ))
            })?;
        let invoke_idx = instance
            .get_export(store.as_context_mut(), Some(&iface_idx), "invoke")
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "skill {} exports tool-provider but no invoke func",
                    manifest.name
                ))
            })?;
        let invoke_func = instance
            .get_func(store.as_context_mut(), invoke_idx)
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "skill {} invoke export is not a function",
                    manifest.name
                ))
            })?;

        // Build the `tool-call-request` record value:
        //   { call-id: string, tool-id: string, args-json: string }
        let request_val = Val::Record(vec![
            ("call-id".into(), Val::String(req.call_id.clone())),
            (
                "tool-id".into(),
                Val::String(if req.tool_id.is_empty() {
                    manifest.name.clone()
                } else {
                    req.tool_id.clone()
                }),
            ),
            ("args-json".into(), Val::String(req.arguments.clone())),
        ]);
        let mut results = [Val::Bool(false)];
        invoke_func
            .call_async(store.as_context_mut(), &[request_val], &mut results)
            .await
            .map_err(|e| classify_call_err(&manifest.name, wall_deadline_ms, e))?;
        invoke_func
            .post_return_async(store.as_context_mut())
            .await
            .ok();

        let Val::Result(result_outcome) = &results[0] else {
            return Err(SandboxError::Internal(format!(
                "skill {} returned non-result value",
                manifest.name
            )));
        };
        match result_outcome.as_ref() {
            Ok(Some(payload)) => decode_tool_result(&manifest.name, &req.call_id, payload),
            Ok(None) => Err(SandboxError::Internal(format!(
                "skill {} returned ok with no value",
                manifest.name
            ))),
            Err(Some(err)) => Err(decode_domain_error(&manifest.name, err)),
            Err(None) => Err(SandboxError::Internal(format!(
                "skill {} returned err with no payload",
                manifest.name
            ))),
        }
    }
}

fn decode_tool_result(
    skill: &str,
    expected_call_id: &str,
    payload: &Val,
) -> std::result::Result<ToolCallResp, SandboxError> {
    let Val::Record(fields) = payload else {
        return Err(SandboxError::Internal(format!(
            "skill {skill} returned non-record success"
        )));
    };
    let mut call_id = String::new();
    let mut tool_id = String::new();
    let mut output_json = String::new();
    let mut is_error = false;
    for (k, v) in fields {
        match (k.as_str(), v) {
            ("call-id", Val::String(s)) => call_id = s.clone(),
            ("tool-id", Val::String(s)) => tool_id = s.clone(),
            ("output-json", Val::String(s)) => output_json = s.clone(),
            ("is-error", Val::Bool(b)) => is_error = *b,
            _ => {}
        }
    }
    if call_id.is_empty() {
        call_id = expected_call_id.to_string();
    }
    if tool_id.is_empty() {
        tool_id = skill.to_string();
    }
    Ok(ToolCallResp {
        call_id,
        tool_id,
        output_json,
        is_error,
    })
}

fn decode_domain_error(skill: &str, err: &Val) -> SandboxError {
    // mycelium:types/types.domain-error is a variant of (Not-Found |
    // Validation | Conflict | Backend | Internal) each with a string. We
    // collapse them all to SandboxError::Trap for the runner's purposes.
    let msg = match err {
        Val::Variant(case, payload) => {
            let pl = payload
                .as_ref()
                .and_then(|v| match v.as_ref() {
                    Val::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            if pl.is_empty() {
                case.clone()
            } else {
                format!("{case}: {pl}")
            }
        }
        _ => format!("{err:?}"),
    };
    SandboxError::Trap(skill.to_string(), msg)
}

fn classify_instantiate_err(skill: &str, e: anyhow::Error) -> SandboxError {
    let msg = format!("{e:#}");
    if msg.contains("import")
        && (msg.contains("not found") || msg.contains("unsatisfied") || msg.contains("missing"))
    {
        SandboxError::CapabilityRefused(skill.to_string(), msg)
    } else {
        SandboxError::Internal(format!("instantiate {skill}: {msg}"))
    }
}

fn classify_call_err(skill: &str, wall_ms: u64, e: anyhow::Error) -> SandboxError {
    let msg = format!("{e:#}");
    if msg.contains("epoch") || msg.contains("interrupt") {
        SandboxError::Timeout(skill.to_string(), wall_ms)
    } else if msg.contains("fuel") {
        SandboxError::OutOfFuel(skill.to_string())
    } else if msg.contains("memory") && msg.contains("grow") {
        SandboxError::OutOfMemory(skill.to_string(), 0)
    } else {
        SandboxError::Trap(skill.to_string(), msg)
    }
}

// Silence unused-import warnings if the no-wasi-cap path ever bakes out.
fn _force_anyhow_use() -> anyhow::Result<()> {
    Err(anyhow!("never called"))
}
