//! Hook runner — NATS request-reply responder over `mycelium.hook.>`.
//!
//! Subject layout: `mycelium.hook.<event-name>` (e.g.
//! `mycelium.hook.before-tool-call`). Callers use `nats.request(...)` so the
//! reply inbox is auto-managed. Payload is opaque JSON; the hook component
//! is free to mutate and return it.
//!
//! If no hook manifest is registered for the event, the runner just echoes
//! the payload back unchanged — this keeps mycelium-core's hook-firing
//! sites idempotent regardless of operator configuration.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_nats::jetstream;
use futures_util::StreamExt;
use mycelium_hook_runner::{
    config::Config,
    registry::HookRegistry,
    sandbox::Sandbox,
    Loader,
};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

const HOOK_SUBJECT: &str = "mycelium.hook.>";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let config = Arc::new(Config::load()?);
    tracing::info!(
        nats_url = %config.nats_url,
        cache = %config.hook_cache_dir,
        caps = ?config.approved_capabilities,
        "mycelium-hook-runner starting"
    );

    let nats = async_nats::connect(&config.nats_url)
        .await
        .with_context(|| format!("connect NATS at {}", config.nats_url))?;
    tracing::info!("event: connected");
    let js = jetstream::new(nats.clone());

    let registry = Arc::new(HookRegistry::open(&js).await?);
    let loader = Arc::new(Loader::new(&config.hook_cache_dir)?);
    let sandbox = Arc::new(Sandbox::new(config.clone())?);

    let mut sub = nats
        .subscribe(HOOK_SUBJECT.to_string())
        .await
        .with_context(|| format!("subscribe {HOOK_SUBJECT}"))?;

    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
    let _shutdown_tx = shutdown_tx;
    tracing::info!("mycelium-hook-runner ready");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("ctrl-c received, draining");
                break;
            }
            _ = shutdown_rx.recv() => break,
            Some(msg) = sub.next() => {
                let registry = registry.clone();
                let loader = loader.clone();
                let sandbox = sandbox.clone();
                let nats = nats.clone();
                let js = js.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_hook(&registry, &loader, &sandbox, &nats, &js, &msg).await {
                        tracing::warn!(subject = %msg.subject, error = %e, "hook handler failed");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle_hook(
    registry: &HookRegistry,
    loader: &Loader,
    sandbox: &Sandbox,
    nats: &async_nats::Client,
    js: &jetstream::Context,
    msg: &async_nats::Message,
) -> Result<()> {
    let event_name = match msg.subject.strip_prefix("mycelium.hook.") {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return Ok(()),
    };
    let payload_str = std::str::from_utf8(&msg.payload).unwrap_or("{}").to_string();
    tracing::debug!(event = %event_name, "dispatching hook");

    let reply_payload = match registry.get(&event_name).await {
        Ok(Some(manifest)) => {
            match loader.fetch(&manifest, js).await {
                Ok(bytes) => match sandbox.invoke(&manifest, &bytes, &event_name, &payload_str).await {
                    Ok(out) => out,
                    Err(e) => {
                        tracing::warn!(event = %event_name, error = %e, "hook invoke failed; passing payload through");
                        payload_str.clone()
                    }
                },
                Err(e) => {
                    tracing::warn!(event = %event_name, error = %e, "hook loader failed; passing payload through");
                    payload_str.clone()
                }
            }
        }
        Ok(None) => {
            // No hook registered for this event — pass-through.
            payload_str.clone()
        }
        Err(e) => {
            tracing::warn!(event = %event_name, error = %e, "hook registry lookup failed; passing payload through");
            payload_str.clone()
        }
    };

    if let Some(reply) = msg.reply.as_ref() {
        nats.publish(reply.clone(), reply_payload.into_bytes().into())
            .await
            .context("publish hook reply")?;
    }
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,mycelium_hook_runner=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
