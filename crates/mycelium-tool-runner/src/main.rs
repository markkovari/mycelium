use std::sync::Arc;

use anyhow::{Context, Result};
use async_nats::jetstream;
use futures_util::StreamExt;
use mycelium_tool_runner::{
    config::Config,
    loader::Loader,
    manifest::SkillManifest,
    registry::SkillRegistry,
    sandbox::{Sandbox, ToolCallReq, ToolCallResp},
};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

const TOOL_CALL_PREFIX: &str = "mycelium.tool.call";
const TOOL_RESULT: &str = "mycelium.tool.result";

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
    let loader = Arc::new(Loader::new(&config.skill_cache_dir)?);
    let sandbox = Arc::new(Sandbox::new(config.clone())?);

    let mut sub = nats
        .subscribe(format!("{TOOL_CALL_PREFIX}.>"))
        .await
        .with_context(|| format!("subscribe {TOOL_CALL_PREFIX}.>"))?;
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
            Some(msg) = sub.next() => {
                let registry = registry.clone();
                let loader = loader.clone();
                let sandbox = sandbox.clone();
                let nats = nats.clone();
                let js = js.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_call(&registry, &loader, &sandbox, &nats, &js, &msg).await {
                        tracing::warn!(error = %e, subject = %msg.subject, "tool call dispatch failed");
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
    let req: ToolCallReq = serde_json::from_slice(&msg.payload).context("decode tool.call")?;
    tracing::debug!(skill = %name, call_id = %req.call_id, "dispatching");

    let resp = match registry.get(&name).await {
        Ok(Some(manifest)) => invoke_one(&manifest, loader, sandbox, js, req).await,
        Ok(None) => ToolCallResp {
            call_id: req.call_id.clone(),
            tool_id: name.clone(),
            output_json: format!(
                r#"{{"error":"skill {} not registered (operator must register manifest)"}}"#,
                name
            ),
            is_error: true,
        },
        Err(e) => ToolCallResp {
            call_id: req.call_id.clone(),
            tool_id: name.clone(),
            output_json: format!(r#"{{"error":"registry lookup failed: {}"}}"#, e),
            is_error: true,
        },
    };

    publish_result(nats, &resp).await?;
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
        Err(e) => {
            return ToolCallResp {
                call_id: req.call_id,
                tool_id: manifest.name.clone(),
                output_json: format!(r#"{{"error":"loader failed: {}"}}"#, e),
                is_error: true,
            };
        }
    };
    match sandbox.invoke(manifest, &bytes, req).await {
        Ok(r) => r,
        Err(e) => ToolCallResp {
            call_id: Default::default(),
            tool_id: manifest.name.clone(),
            output_json: format!(r#"{{"error":"sandbox: {}"}}"#, e),
            is_error: true,
        },
    }
}

async fn publish_result(nats: &async_nats::Client, resp: &ToolCallResp) -> Result<()> {
    // Executor expects {task_id, call_id, output, error?}. ToolCallResp here
    // mirrors the existing schema except for keying (we don't know task_id;
    // executor recovers that from call_id via its in-flight task state).
    // For backwards-compat with the existing executor we publish a payload
    // that contains both call_id and output/error.
    let body = serde_json::json!({
        "call_id": resp.call_id,
        "tool_id": resp.tool_id,
        "output": if resp.is_error { serde_json::Value::Null } else { serde_json::Value::String(resp.output_json.clone()) },
        "error": if resp.is_error { serde_json::Value::String(resp.output_json.clone()) } else { serde_json::Value::Null },
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
