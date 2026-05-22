use std::sync::Arc;

use anyhow::{Context, Result};
use async_nats::jetstream;
use futures_util::StreamExt;
use mycelium_tool_runner::{
    config::Config,
    mcp_registry::McpRegistry,
    registry::SkillRegistry,
    sandbox::{Sandbox, ToolCallReq, ToolCallResp},
    Loader, SkillManifest,
};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

const TOOL_CALL_PREFIX: &str = "mycelium.tool.call";
const TOOL_RESULT: &str = "mycelium.tool.result";
const MCP_INSTALL: &str = "mycelium.mcp.install";
const MCP_UNINSTALL: &str = "mycelium.mcp.uninstall";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let config = Arc::new(Config::load()?);
    tracing::info!(
        nats_url = %config.nats_url,
        cache = %config.skill_cache_dir,
        caps = ?config.approved_capabilities,
        "mycelium-tool-runner starting",
    );

    let nats = async_nats::connect(&config.nats_url)
        .await
        .with_context(|| format!("connect to NATS at {}", config.nats_url))?;
    let js = jetstream::new(nats.clone());
    let registry = Arc::new(SkillRegistry::open(&js).await?);
    let mcp_registry = Arc::new(McpRegistry::open(&js).await?);
    let loader = Arc::new(Loader::new(&config.skill_cache_dir)?);
    let sandbox = Arc::new(Sandbox::new(config.clone(), Arc::new(js.clone()))?);

    let mut tool_sub = nats
        .subscribe(format!("{TOOL_CALL_PREFIX}.>"))
        .await
        .with_context(|| format!("subscribe {TOOL_CALL_PREFIX}.>"))?;
    let mut install_sub = nats
        .subscribe(MCP_INSTALL)
        .await
        .with_context(|| format!("subscribe {MCP_INSTALL}"))?;
    let mut uninstall_sub = nats
        .subscribe(MCP_UNINSTALL)
        .await
        .with_context(|| format!("subscribe {MCP_UNINSTALL}"))?;

    let (shutdown_tx, _) = broadcast::channel::<()>(8);
    let mut shutdown_rx = shutdown_tx.subscribe();

    tracing::info!("mycelium-tool-runner ready");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("ctrl-c received, shutting down");
                let _ = shutdown_tx.send(());
                break;
            }
            _ = shutdown_rx.recv() => {
                break;
            }
            Some(msg) = tool_sub.next() => {
                let registry = registry.clone();
                let mcp_registry = mcp_registry.clone();
                let loader = loader.clone();
                let sandbox = sandbox.clone();
                let nats = nats.clone();
                let js = js.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_call(
                        &registry, &mcp_registry, &loader, &sandbox, &nats, &js, &msg,
                    ).await {
                        tracing::warn!(error = %e, subject = %msg.subject, "tool call dispatch failed");
                    }
                });
            }
            Some(msg) = install_sub.next() => {
                let mcp_registry = mcp_registry.clone();
                let loader = loader.clone();
                let sandbox = sandbox.clone();
                let js = js.clone();
                tokio::spawn(async move {
                    match serde_json::from_slice::<SkillManifest>(&msg.payload) {
                        Ok(manifest) => {
                            match mcp_registry.install(&manifest, &loader, &sandbox, &js).await {
                                Ok(tools) => tracing::info!(
                                    server = %manifest.name,
                                    tools = tools.len(),
                                    "MCP install ok",
                                ),
                                Err(e) => tracing::warn!(
                                    server = %manifest.name,
                                    error = %e,
                                    "MCP install failed",
                                ),
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "invalid MCP install payload"),
                    }
                });
            }
            Some(msg) = uninstall_sub.next() => {
                let mcp_registry = mcp_registry.clone();
                tokio::spawn(async move {
                    #[derive(serde::Deserialize)]
                    struct Req { name: String }
                    match serde_json::from_slice::<Req>(&msg.payload) {
                        Ok(req) => {
                            if let Err(e) = mcp_registry.uninstall(&req.name).await {
                                tracing::warn!(server = %req.name, error = %e, "MCP uninstall failed");
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "invalid MCP uninstall payload"),
                    }
                });
            }
        }
    }
    tracing::info!("mycelium-tool-runner stopped");
    Ok(())
}

async fn handle_call(
    registry: &SkillRegistry,
    mcp_registry: &McpRegistry,
    loader: &Loader,
    sandbox: &Sandbox,
    nats: &async_nats::Client,
    js: &jetstream::Context,
    msg: &async_nats::Message,
) -> Result<()> {
    let name = match msg.subject.strip_prefix(&format!("{TOOL_CALL_PREFIX}.")) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return Ok(()),
    };
    let envelope: serde_json::Value =
        serde_json::from_slice(&msg.payload).context("decode tool.call")?;
    let task_id = envelope
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let call_id = envelope
        .get("call_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let arguments = envelope
        .get("arguments")
        .and_then(|v| v.as_str())
        .unwrap_or("{}")
        .to_string();
    let req = ToolCallReq {
        call_id: call_id.clone(),
        tool_id: name.clone(),
        arguments: arguments.clone(),
    };
    tracing::debug!(tool = %name, task_id = %task_id, call_id = %call_id, "dispatching");

    // 1. Try skill registry (existing 1:1 components).
    let resp = if let Ok(Some(manifest)) = registry.get(&name).await {
        invoke_one(&manifest, loader, sandbox, js, req).await
    // 2. Try MCP registry (1:N components).
    } else if let Some(server_name) = mcp_registry.route(&name).await {
        match mcp_registry.get_manifest(&server_name).await {
            Ok(Some(manifest)) => invoke_mcp_tool(&manifest, loader, sandbox, js, &name, &arguments, &call_id).await,
            Ok(None) => error_resp(&call_id, &name, &format!("MCP server {server_name} manifest missing")),
            Err(e) => error_resp(&call_id, &name, &format!("MCP manifest lookup failed: {e}")),
        }
    // 3. Not found.
    } else {
        error_resp(
            &call_id,
            &name,
            &format!("tool {name} not registered (no skill or MCP server found)"),
        )
    };

    publish_result(nats, &task_id, &resp).await?;
    Ok(())
}

async fn invoke_one(
    manifest: &SkillManifest,
    loader: &Loader,
    sandbox: &Sandbox,
    js: &jetstream::Context,
    req: ToolCallReq,
) -> ToolCallResp {
    let bytes = match loader.fetch(manifest, js).await {
        Ok(b) => b,
        Err(e) => return error_resp(&req.call_id, &manifest.name, &format!("loader failed: {e}")),
    };
    match sandbox.invoke(manifest, &bytes, req).await {
        Ok(r) => r,
        Err(e) => error_resp("", &manifest.name, &format!("sandbox: {e}")),
    }
}

async fn invoke_mcp_tool(
    manifest: &SkillManifest,
    loader: &Loader,
    sandbox: &Sandbox,
    js: &jetstream::Context,
    tool_name: &str,
    arguments_json: &str,
    call_id: &str,
) -> ToolCallResp {
    let bytes = match loader.fetch(manifest, js).await {
        Ok(b) => b,
        Err(e) => return error_resp(call_id, tool_name, &format!("loader failed: {e}")),
    };
    match sandbox.invoke_mcp(manifest, &bytes, tool_name, arguments_json, call_id).await {
        Ok(r) => r,
        Err(e) => error_resp(call_id, tool_name, &format!("sandbox: {e}")),
    }
}

fn error_resp(call_id: &str, tool_id: &str, msg: &str) -> ToolCallResp {
    ToolCallResp {
        call_id: call_id.to_string(),
        tool_id: tool_id.to_string(),
        output_json: format!(r#"{{"error":{}}}"#, serde_json::json!(msg)),
        is_error: true,
    }
}

async fn publish_result(
    nats: &async_nats::Client,
    task_id: &str,
    resp: &ToolCallResp,
) -> Result<()> {
    let body = serde_json::json!({
        "task_id": task_id,
        "call_id": resp.call_id,
        "tool_id": resp.tool_id,
        "output": if resp.is_error { serde_json::Value::Null } else { serde_json::Value::String(resp.output_json.clone()) },
        "error":  if resp.is_error { serde_json::Value::String(resp.output_json.clone()) } else { serde_json::Value::Null },
    });
    let bytes = serde_json::to_vec(&body)?;
    nats.publish(TOOL_RESULT.to_string(), bytes.into()).await?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,mycelium_tool_runner=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
