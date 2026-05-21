//! Telegram transport adapter — long polling.
//!
//! Native rewrite of `components/telegram-poller/src/lib.rs`. Single tokio
//! task: long-poll Telegram with `timeout=25`, publish each update to
//! `mycelium.channel.telegram.raw`, advance offset in
//! `mycelium-telegram-poller-state` KV.
//!
//! No more lease lock, no more tick re-arm cooldown, no more lock-until KV
//! shenanigans. Those existed only because wash 2.1.0 fanned every tick to
//! N parallel component instances, causing exponential storms. Native runs
//! one instance; the loop is the only loop.

use anyhow::{Context, Result};
use async_nats::jetstream::kv::Store;
use tokio::sync::broadcast;

use crate::{state::AppState, telegram::TgClient};

const STATE_BUCKET: &str = "mycelium-telegram-poller-state";
const OFFSET_KEY: &str = "telegram/offset";
const RAW_SUBJECT: &str = "mycelium.channel.telegram.raw";
const ACK_EMOJI: &str = "\u{1F440}"; // 👀

pub async fn run(state: AppState, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
    if state.config.telegram_bot_token.is_empty() {
        tracing::warn!("telegram.bot_token not configured; telegram_poller disabled");
        return Ok(());
    }
    let kv = state
        .js
        .get_key_value(STATE_BUCKET)
        .await
        .with_context(|| format!("open {STATE_BUCKET}"))?;
    let client = TgClient::new(state.config.telegram_bot_token.clone(), state.http.clone());

    tracing::info!("telegram_poller started");

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                tracing::info!("telegram_poller shutting down");
                break;
            }
            res = tick_once(&state, &kv, &client) => {
                if let Err(e) = res {
                    tracing::warn!(error = %e, "tick failed; backing off 2s");
                    sleep_with_shutdown(std::time::Duration::from_secs(2), &mut shutdown).await;
                }
            }
        }
    }
    Ok(())
}

async fn tick_once(state: &AppState, kv: &Store, client: &TgClient) -> Result<()> {
    let offset = read_offset(kv).await.unwrap_or(0);
    let updates = client.get_updates(offset, 25).await?;
    for u in &updates {
        let bytes = serde_json::to_vec(u)?;
        state
            .nats
            .publish(RAW_SUBJECT.to_string(), bytes.into())
            .await?;
        // best-effort 👀 ack on the user's message
        if let Some(msg) = u.message.as_ref() {
            if let Err(e) = client.set_reaction(msg.chat.id, msg.message_id, ACK_EMOJI).await {
                tracing::debug!(error = %e, "set_reaction failed (non-fatal)");
            }
        }
    }
    if let Some(max) = updates.iter().map(|u| u.update_id).max() {
        write_offset(kv, max + 1).await?;
    }
    if updates.is_empty() {
        // Empty long-poll already burned the 25s timeout; small extra idle
        // sleep keeps us friendly when Telegram returns instantly on errors.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Ok(())
}

async fn read_offset(kv: &Store) -> Result<u64> {
    let Some(bytes) = kv.get(OFFSET_KEY).await? else {
        return Ok(0);
    };
    let s = std::str::from_utf8(&bytes)?;
    Ok(s.parse().unwrap_or(0))
}

async fn write_offset(kv: &Store, offset: u64) -> Result<()> {
    kv.put(OFFSET_KEY, offset.to_string().into_bytes().into()).await?;
    Ok(())
}

async fn sleep_with_shutdown(d: std::time::Duration, shutdown: &mut broadcast::Receiver<()>) {
    tokio::select! {
        _ = tokio::time::sleep(d) => {}
        _ = shutdown.recv() => {}
    }
}
