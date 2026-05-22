//! wasmtime sandbox host for skills and MCP server components.
//!
//! Per-invocation lifecycle:
//! 1. Validate manifest capabilities against the operator allow-list.
//! 2. Build a fresh wasmtime::Store with fuel, an epoch deadline and a
//!    memory cap derived from the manifest (or runner defaults).
//! 3. Construct a Linker that only adds host imports the manifest declared,
//!    so a `calc` skill physically cannot reach wasi:http.
//! 4. Instantiate the component and dispatch to the appropriate export:
//!    - `mycelium:tool/tool-provider.invoke`  (Skill)
//!    - `mycelium:mcp/mcp-provider.list-tools` / `.call-tool`  (McpServer)

use anyhow::{anyhow, Context as _, Result};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use wasmtime::component::{Component, Instance, Linker, Resource, Val};
use wasmtime::{AsContextMut, Engine, Store, StoreLimitsBuilder};
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use async_trait::async_trait;
use crate::kv_host::{wasi::keyvalue::store as kv_store, NatsBucket};
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

/// Tool definition returned by an MCP component's `list-tools` call.
#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema string (OpenAI function-calling `parameters` shape).
    pub input_schema: String,
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

/// Per-invocation store state.
pub struct SkillCtx {
    pub wasi: Option<wasmtime_wasi::WasiCtx>,
    pub http: WasiHttpCtx,
    pub table: ResourceTable,
    pub limits: wasmtime::StoreLimits,
    /// JetStream context, present only when the component declared wasi:keyvalue capability.
    pub js: Option<Arc<async_nats::jetstream::Context>>,
}

impl wasmtime_wasi::WasiView for SkillCtx {
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx {
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

// ── wasi:keyvalue/store host impl ────────────────────────────────────────────

#[async_trait]
impl kv_store::Host for SkillCtx {
    async fn open(
        &mut self,
        identifier: String,
    ) -> Result<Resource<NatsBucket>, kv_store::Error> {
        let js = self
            .js
            .as_ref()
            .ok_or(kv_store::Error::AccessDenied)?
            .clone();
        let kv = match js.get_key_value(&identifier).await {
            Ok(kv) => kv,
            Err(_) => js
                .create_key_value(async_nats::jetstream::kv::Config {
                    bucket: identifier.clone(),
                    ..Default::default()
                })
                .await
                .map_err(|e| kv_store::Error::Other(e.to_string()))?,
        };
        self.table
            .push(NatsBucket { kv })
            .map_err(|e| kv_store::Error::Other(e.to_string()))
    }
}

#[async_trait]
impl kv_store::HostBucket for SkillCtx {
    async fn get(
        &mut self,
        bucket: Resource<NatsBucket>,
        key: String,
    ) -> Result<Option<Vec<u8>>, kv_store::Error> {
        let kv = self
            .table
            .get(&bucket)
            .map_err(|e| kv_store::Error::Other(e.to_string()))?
            .kv
            .clone();
        match kv.get(&key).await.map_err(|e| kv_store::Error::Other(e.to_string()))? {
            Some(bytes) => Ok(Some(bytes.to_vec())),
            None => Ok(None),
        }
    }

    async fn set(
        &mut self,
        bucket: Resource<NatsBucket>,
        key: String,
        value: Vec<u8>,
    ) -> Result<(), kv_store::Error> {
        let kv = self
            .table
            .get(&bucket)
            .map_err(|e| kv_store::Error::Other(e.to_string()))?
            .kv
            .clone();
        kv.put(&key, value.into())
            .await
            .map_err(|e| kv_store::Error::Other(e.to_string()))?;
        Ok(())
    }

    async fn delete(
        &mut self,
        bucket: Resource<NatsBucket>,
        key: String,
    ) -> Result<(), kv_store::Error> {
        let kv = self
            .table
            .get(&bucket)
            .map_err(|e| kv_store::Error::Other(e.to_string()))?
            .kv
            .clone();
        kv.delete(&key)
            .await
            .map_err(|e| kv_store::Error::Other(e.to_string()))?;
        Ok(())
    }

    async fn exists(
        &mut self,
        bucket: Resource<NatsBucket>,
        key: String,
    ) -> Result<bool, kv_store::Error> {
        let kv = self
            .table
            .get(&bucket)
            .map_err(|e| kv_store::Error::Other(e.to_string()))?
            .kv
            .clone();
        kv.get(&key)
            .await
            .map(|v| v.is_some())
            .map_err(|e| kv_store::Error::Other(e.to_string()))
    }

    async fn list_keys(
        &mut self,
        bucket: Resource<NatsBucket>,
        _cursor: Option<u64>,
    ) -> Result<kv_store::KeyResponse, kv_store::Error> {
        let kv = self
            .table
            .get(&bucket)
            .map_err(|e| kv_store::Error::Other(e.to_string()))?
            .kv
            .clone();
        let keys = kv
            .keys()
            .await
            .map_err(|e| kv_store::Error::Other(e.to_string()))?
            .filter_map(|r| async move { r.ok() })
            .collect::<Vec<_>>()
            .await;
        Ok(kv_store::KeyResponse { keys, cursor: None })
    }

    async fn drop(&mut self, rep: Resource<NatsBucket>) -> Result<()> {
        self.table.delete(rep)?;
        Ok(())
    }
}

// ── Sandbox ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Sandbox {
    config: Arc<Config>,
    engine: Engine,
    js: Arc<async_nats::jetstream::Context>,
}

impl Sandbox {
    pub fn new(config: Arc<Config>, js: Arc<async_nats::jetstream::Context>) -> Result<Self> {
        let mut wc = wasmtime::Config::new();
        wc.async_support(true);
        wc.consume_fuel(true);
        wc.epoch_interruption(true);
        wc.wasm_component_model(true);
        let engine = Engine::new(&wc)?;

        // Background ticker for epoch_interruption. Increment once per
        // millisecond; Store deadlines expressed as epoch ticks fire at that
        // resolution. Ticker lives for the process lifetime.
        let engine_for_tick = engine.clone();
        std::thread::Builder::new()
            .name("wasmtime-epoch".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(1));
                engine_for_tick.increment_epoch();
            })
            .context("spawn wasmtime epoch ticker thread")?;

        Ok(Self { config, engine, js })
    }

    // ── Skill (tool-provider) ────────────────────────────────────────────────

    pub async fn invoke(
        &self,
        manifest: &SkillManifest,
        wasm_bytes: &[u8],
        req: ToolCallReq,
    ) -> std::result::Result<ToolCallResp, SandboxError> {
        let wall_deadline_ms = manifest
            .wall_deadline_ms
            .unwrap_or(self.config.default_wall_deadline_ms);
        let (mut store, instance) = self.setup(manifest, wasm_bytes).await?;

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
        invoke_func.post_return_async(store.as_context_mut()).await.ok();

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

    // ── MCP server (mcp-provider) ────────────────────────────────────────────

    /// Call `mycelium:mcp/mcp-provider.list-tools` on a freshly instantiated
    /// MCP component. Used at install time to discover the tool catalogue.
    pub async fn list_tools_mcp(
        &self,
        manifest: &SkillManifest,
        wasm_bytes: &[u8],
    ) -> std::result::Result<Vec<McpToolDef>, SandboxError> {
        let wall_deadline_ms = manifest
            .wall_deadline_ms
            .unwrap_or(self.config.default_wall_deadline_ms);
        let (mut store, instance) = self.setup(manifest, wasm_bytes).await?;

        let iface_idx = instance
            .get_export(store.as_context_mut(), None, "mycelium:mcp/mcp-provider@0.1.0")
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "component {} does not export mycelium:mcp/mcp-provider@0.1.0",
                    manifest.name
                ))
            })?;
        let fn_idx = instance
            .get_export(store.as_context_mut(), Some(&iface_idx), "list-tools")
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "MCP component {} has no list-tools export",
                    manifest.name
                ))
            })?;
        let list_fn = instance
            .get_func(store.as_context_mut(), fn_idx)
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "MCP component {} list-tools is not a function",
                    manifest.name
                ))
            })?;

        let mut results = [Val::Bool(false)];
        list_fn
            .call_async(store.as_context_mut(), &[], &mut results)
            .await
            .map_err(|e| classify_call_err(&manifest.name, wall_deadline_ms, e))?;
        list_fn.post_return_async(store.as_context_mut()).await.ok();

        let Val::List(items) = &results[0] else {
            return Err(SandboxError::Internal(format!(
                "MCP component {} list-tools returned non-list",
                manifest.name
            )));
        };

        let mut tools = Vec::with_capacity(items.len());
        for item in items.iter() {
            let Val::Record(fields) = item else { continue };
            let mut name = String::new();
            let mut description = String::new();
            let mut input_schema = String::new();
            for (k, v) in fields {
                match (k.as_str(), v) {
                    ("name", Val::String(s)) => name = s.clone(),
                    ("description", Val::String(s)) => description = s.clone(),
                    ("input-schema", Val::String(s)) => input_schema = s.clone(),
                    _ => {}
                }
            }
            if !name.is_empty() {
                tools.push(McpToolDef { name, description, input_schema });
            }
        }
        Ok(tools)
    }

    /// Call `mycelium:mcp/mcp-provider.call-tool` on an MCP component.
    pub async fn invoke_mcp(
        &self,
        manifest: &SkillManifest,
        wasm_bytes: &[u8],
        tool_name: &str,
        arguments_json: &str,
        call_id: &str,
    ) -> std::result::Result<ToolCallResp, SandboxError> {
        let wall_deadline_ms = manifest
            .wall_deadline_ms
            .unwrap_or(self.config.default_wall_deadline_ms);
        let (mut store, instance) = self.setup(manifest, wasm_bytes).await?;

        let iface_idx = instance
            .get_export(store.as_context_mut(), None, "mycelium:mcp/mcp-provider@0.1.0")
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "component {} does not export mycelium:mcp/mcp-provider@0.1.0",
                    manifest.name
                ))
            })?;
        let fn_idx = instance
            .get_export(store.as_context_mut(), Some(&iface_idx), "call-tool")
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "MCP component {} has no call-tool export",
                    manifest.name
                ))
            })?;
        let call_fn = instance
            .get_func(store.as_context_mut(), fn_idx)
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "MCP component {} call-tool is not a function",
                    manifest.name
                ))
            })?;

        let params = [
            Val::String(tool_name.to_string()),
            Val::String(arguments_json.to_string()),
        ];
        let mut results = [Val::Bool(false)];
        call_fn
            .call_async(store.as_context_mut(), &params, &mut results)
            .await
            .map_err(|e| classify_call_err(&manifest.name, wall_deadline_ms, e))?;
        call_fn.post_return_async(store.as_context_mut()).await.ok();

        // result<mcp-result, string>
        let Val::Result(outcome) = &results[0] else {
            return Err(SandboxError::Internal(format!(
                "MCP component {} call-tool returned non-result",
                manifest.name
            )));
        };
        match outcome.as_ref() {
            Ok(Some(payload)) => {
                let Val::Record(fields) = payload.as_ref() else {
                    return Err(SandboxError::Internal(format!(
                        "MCP component {} call-tool ok payload is not a record",
                        manifest.name
                    )));
                };
                let mut output_json = String::new();
                let mut is_error = false;
                for (k, v) in fields {
                    match (k.as_str(), v) {
                        ("output-json", Val::String(s)) => output_json = s.clone(),
                        ("is-error", Val::Bool(b)) => is_error = *b,
                        _ => {}
                    }
                }
                Ok(ToolCallResp {
                    call_id: call_id.to_string(),
                    tool_id: tool_name.to_string(),
                    output_json,
                    is_error,
                })
            }
            Ok(None) => Err(SandboxError::Internal(format!(
                "MCP component {} call-tool returned ok with no payload",
                manifest.name
            ))),
            Err(Some(boxed)) => {
                let msg = match boxed.as_ref() {
                    Val::String(s) => s.clone(),
                    other => format!("{other:?}"),
                };
                Err(SandboxError::Trap(manifest.name.clone(), msg))
            }
            Err(None) => Err(SandboxError::Trap(manifest.name.clone(), "call-tool error".into())),
        }
    }

    // ── Shared setup ─────────────────────────────────────────────────────────

    /// Build a sandboxed wasmtime Store + instantiated component. Shared by all
    /// dispatch methods so capability-gating, resource limits, and WASI setup
    /// are applied identically regardless of which export is called.
    async fn setup(
        &self,
        manifest: &SkillManifest,
        wasm_bytes: &[u8],
    ) -> std::result::Result<(Store<SkillCtx>, Instance), SandboxError> {
        for cap in &manifest.capabilities {
            if !self.config.capability_approved(cap) {
                return Err(SandboxError::CapabilityRefused(
                    manifest.name.clone(),
                    cap.clone(),
                ));
            }
        }

        let fuel = manifest.fuel_limit.unwrap_or(self.config.default_fuel_limit);
        let mem_max = manifest.memory_max_bytes.unwrap_or(self.config.default_memory_max);
        let wall_deadline_ms = manifest
            .wall_deadline_ms
            .unwrap_or(self.config.default_wall_deadline_ms);

        let use_kv = manifest.capabilities.iter().any(|c| c.starts_with("wasi:keyvalue"));

        let component = Component::from_binary(&self.engine, wasm_bytes)
            .map_err(|e| SandboxError::Internal(format!("load component: {e}")))?;

        let mut linker: Linker<SkillCtx> = Linker::new(&self.engine);
        wasmtime_wasi::add_to_linker_async(&mut linker)
            .map_err(|e| SandboxError::Internal(format!("link wasi: {e}")))?;
        if manifest.capabilities.iter().any(|c| c.starts_with("wasi:http")) {
            wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)
                .map_err(|e| SandboxError::Internal(format!("link wasi:http: {e}")))?;
        }
        if use_kv {
            crate::kv_host::add_to_linker(&mut linker)
                .map_err(|e| SandboxError::Internal(format!("link wasi:keyvalue: {e}")))?;
        }

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
                js: if use_kv { Some(Arc::clone(&self.js)) } else { None },
            },
        );
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(fuel)
            .map_err(|e| SandboxError::Internal(format!("set fuel: {e}")))?;
        store.set_epoch_deadline(wall_deadline_ms.max(1));

        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .map_err(|e| classify_instantiate_err(&manifest.name, e))?;

        Ok((store, instance))
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
    Ok(ToolCallResp { call_id, tool_id, output_json, is_error })
}

fn decode_domain_error(skill: &str, err: &Val) -> SandboxError {
    let msg = match err {
        Val::Variant(case, payload) => {
            let pl = payload
                .as_ref()
                .and_then(|v| match v.as_ref() {
                    Val::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            if pl.is_empty() { case.clone() } else { format!("{case}: {pl}") }
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

fn _force_anyhow_use() -> anyhow::Result<()> {
    Err(anyhow!("never called"))
}
