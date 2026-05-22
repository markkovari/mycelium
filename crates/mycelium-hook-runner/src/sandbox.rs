//! wasmtime sandbox host for hooks.
//!
//! Per-invocation lifecycle mirrors mycelium-tool-runner's sandbox:
//!   1. Validate manifest capabilities against the operator allow-list.
//!   2. Build a fresh `wasmtime::Store` with fuel, epoch deadline, memory cap.
//!   3. Build a Linker that only adds host imports the manifest declared.
//!   4. Instantiate, call `hook-provider.handle(event, payload)`, return the
//!      result string (or the error string the component produced).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use wasmtime::component::{Component, Linker, Val};
use wasmtime::{AsContextMut, Engine, Store, StoreLimitsBuilder};
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use crate::{config::Config, HookManifest};

#[derive(thiserror::Error, Debug)]
pub enum SandboxError {
    #[error("hook {0} requested capability {1} which is not in the operator allow-list")]
    CapabilityRefused(String, String),
    #[error("hook {0} exceeded wall-clock deadline ({1} ms)")]
    Timeout(String, u64),
    #[error("hook {0} ran out of fuel")]
    OutOfFuel(String),
    #[error("hook {0} trapped: {1}")]
    Trap(String, String),
    #[error("hook component {0} returned an error: {1}")]
    HookErr(String, String),
    #[error("sandbox internal error: {0}")]
    Internal(String),
}

pub struct HookCtx {
    pub wasi: Option<wasmtime_wasi::WasiCtx>,
    pub http: WasiHttpCtx,
    pub table: ResourceTable,
    pub limits: wasmtime::StoreLimits,
}

impl wasmtime_wasi::WasiView for HookCtx {
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx {
        self.wasi
            .as_mut()
            .expect("WasiView::ctx called but no WasiCtx was installed for this hook")
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiHttpView for HookCtx {
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

        // Shared epoch ticker — 1ms resolution, lives for the process.
        let engine_for_tick = engine.clone();
        std::thread::Builder::new()
            .name("wasmtime-hook-epoch".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(1));
                engine_for_tick.increment_epoch();
            })
            .context("spawn wasmtime epoch ticker thread")?;

        Ok(Self { config, engine })
    }

    pub async fn invoke(
        &self,
        manifest: &HookManifest,
        wasm_bytes: &[u8],
        event_name: &str,
        payload_json: &str,
    ) -> std::result::Result<String, SandboxError> {
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

        let mut linker: Linker<HookCtx> = Linker::new(&self.engine);
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

        let wasi_ctx = Some(WasiCtxBuilder::new().inherit_stderr().build());
        let limits = StoreLimitsBuilder::new()
            .memory_size(mem_max as usize)
            .build();
        let mut store = Store::new(
            &self.engine,
            HookCtx {
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
        store.set_epoch_deadline(wall_deadline_ms.max(1));

        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .map_err(|e| classify_instantiate_err(&manifest.name, e))?;

        let iface_idx = instance
            .get_export(store.as_context_mut(), None, "mycelium:hook/hook-provider@0.1.0")
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "hook {} does not export mycelium:hook/hook-provider@0.1.0",
                    manifest.name
                ))
            })?;
        let handle_idx = instance
            .get_export(store.as_context_mut(), Some(&iface_idx), "handle")
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "hook {} exports hook-provider but no handle func",
                    manifest.name
                ))
            })?;
        let handle_func = instance
            .get_func(store.as_context_mut(), handle_idx)
            .ok_or_else(|| {
                SandboxError::Internal(format!(
                    "hook {} handle export is not a function",
                    manifest.name
                ))
            })?;

        let args = [
            Val::String(event_name.to_string()),
            Val::String(payload_json.to_string()),
        ];
        let mut results = [Val::Bool(false)];
        handle_func
            .call_async(store.as_context_mut(), &args, &mut results)
            .await
            .map_err(|e| classify_call_err(&manifest.name, wall_deadline_ms, e))?;
        handle_func
            .post_return_async(store.as_context_mut())
            .await
            .ok();

        let Val::Result(outcome) = &results[0] else {
            return Err(SandboxError::Internal(format!(
                "hook {} returned non-result value",
                manifest.name
            )));
        };
        match outcome.as_ref() {
            Ok(Some(boxed)) => match boxed.as_ref() {
                Val::String(s) => Ok(s.clone()),
                _ => Err(SandboxError::Internal(format!(
                    "hook {} ok-branch not a string",
                    manifest.name
                ))),
            },
            Ok(None) => Err(SandboxError::Internal(format!(
                "hook {} returned ok with no value",
                manifest.name
            ))),
            Err(Some(boxed)) => match boxed.as_ref() {
                Val::String(s) => Err(SandboxError::HookErr(manifest.name.clone(), s.clone())),
                _ => Err(SandboxError::Internal(format!(
                    "hook {} err-branch not a string",
                    manifest.name
                ))),
            },
            Err(None) => Err(SandboxError::Internal(format!(
                "hook {} returned err with no payload",
                manifest.name
            ))),
        }
    }
}

fn classify_instantiate_err(hook: &str, e: anyhow::Error) -> SandboxError {
    let msg = format!("{e:#}");
    if msg.contains("import")
        && (msg.contains("not found") || msg.contains("unsatisfied") || msg.contains("missing"))
    {
        SandboxError::CapabilityRefused(hook.to_string(), msg)
    } else {
        SandboxError::Internal(format!("instantiate {hook}: {msg}"))
    }
}

fn classify_call_err(hook: &str, wall_ms: u64, e: anyhow::Error) -> SandboxError {
    let msg = format!("{e:#}");
    if msg.contains("epoch") || msg.contains("interrupt") {
        SandboxError::Timeout(hook.to_string(), wall_ms)
    } else if msg.contains("fuel") {
        SandboxError::OutOfFuel(hook.to_string())
    } else {
        SandboxError::Trap(hook.to_string(), msg)
    }
}
