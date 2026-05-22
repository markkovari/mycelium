//! Outbound Telegram dispatcher.
//!
//! Other modules publish replies via either:
//! * `mycelium.channel.telegram.out.<chat_id>`    JSON `{recipient_id, text}`
//! * `mycelium.channel.telegram.action.<chat_id>` JSON `{action}` (default
//!                                                  "typing")
//!
//! Executor already calls Telegram inline (it owns the progress-edit chain),
//! so this module is mostly a compatibility shim for any external publisher
//! that wants to drive Telegram I/O without rebuilding the executor.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{state::AppState, telegram::TgClient};

const OUT_SUBJECT: &str = "mycelium.channel.telegram.out.>";
const ACTION_SUBJECT: &str = "mycelium.channel.telegram.action.>";
const STREAM_SUBJECT: &str = "mycelium.run.*.assistant";

/// Minimum interval between Telegram editMessageText calls per chat.
/// Telegram itself rate-limits message edits to roughly one per second.
const EDIT_DEBOUNCE: Duration = Duration::from_millis(900);

#[derive(Default)]
struct StreamBuffer {
    buf: String,
    last_edit: Option<Instant>,
    progress_message_id: i64,
}

#[derive(Deserialize)]
struct ChannelReply {
    recipient_id: String,
    text: String,
}

#[derive(Deserialize)]
struct ChannelAction {
    #[serde(default = "default_action")]
    action: String,
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    delta: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    progress_message_id: i64,
}

fn default_action() -> String {
    "typing".to_string()
}

pub async fn run(state: AppState, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
    if state.config.telegram_bot_token.is_empty() {
        tracing::warn!("telegram.bot_token not configured; telegram_out disabled");
        return Ok(());
    }
    let tg = TgClient::new(state.config.telegram_bot_token.clone(), state.http.clone());
    let mut sub_out = state
        .nats
        .subscribe(OUT_SUBJECT.to_string())
        .await
        .with_context(|| format!("subscribe {OUT_SUBJECT}"))?;
    let mut sub_action = state
        .nats
        .subscribe(ACTION_SUBJECT.to_string())
        .await
        .with_context(|| format!("subscribe {ACTION_SUBJECT}"))?;
    let mut sub_stream = state
        .nats
        .subscribe(STREAM_SUBJECT.to_string())
        .await
        .with_context(|| format!("subscribe {STREAM_SUBJECT}"))?;
    tracing::info!("telegram_out started");

    // Per-chat assistant-stream debounce buffers.
    let mut streams: HashMap<String, StreamBuffer> = HashMap::new();

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                tracing::info!("telegram_out shutting down");
                break;
            }
            Some(msg) = sub_out.next() => {
                if let Err(e) = handle_out(&tg, &msg).await {
                    tracing::warn!(error = %e, "telegram_out send failed");
                }
            }
            Some(msg) = sub_action.next() => {
                if let Err(e) = handle_action(&tg, &msg).await {
                    tracing::warn!(error = %e, "telegram_out action failed");
                }
            }
            Some(msg) = sub_stream.next() => {
                if let Err(e) = handle_stream(&tg, &mut streams, &msg).await {
                    tracing::debug!(error = %e, "telegram_out stream chunk skipped");
                }
            }
        }
    }
    Ok(())
}

async fn handle_out(tg: &TgClient, msg: &async_nats::Message) -> Result<()> {
    let reply: ChannelReply = serde_json::from_slice(&msg.payload)?;
    tg.send_message(&reply.recipient_id, &reply.text).await?;
    Ok(())
}

async fn handle_action(tg: &TgClient, msg: &async_nats::Message) -> Result<()> {
    let chat_id = msg
        .subject
        .strip_prefix("mycelium.channel.telegram.action.")
        .unwrap_or("");
    if chat_id.is_empty() {
        return Ok(());
    }
    let act: ChannelAction = if msg.payload.is_empty() {
        ChannelAction { action: default_action() }
    } else {
        serde_json::from_slice(&msg.payload)?
    };
    tg.send_chat_action(chat_id, &act.action).await?;
    Ok(())
}

async fn handle_stream(
    tg: &TgClient,
    streams: &mut HashMap<String, StreamBuffer>,
    msg: &async_nats::Message,
) -> Result<()> {
    let chunk: StreamChunk = serde_json::from_slice(&msg.payload)?;
    if chunk.chat_id.is_empty() || chunk.progress_message_id == 0 {
        return Ok(());
    }
    let entry = streams.entry(chunk.chat_id.clone()).or_default();
    entry.progress_message_id = chunk.progress_message_id;
    if !chunk.delta.is_empty() {
        entry.buf.push_str(&chunk.delta);
    }
    let now = Instant::now();
    let should_edit = chunk.done
        || entry
            .last_edit
            .map(|t| now.duration_since(t) >= EDIT_DEBOUNCE)
            .unwrap_or(true);
    if !should_edit || entry.buf.is_empty() {
        return Ok(());
    }
    let body = if chunk.done {
        std::mem::take(&mut entry.buf)
    } else {
        entry.buf.clone()
    };
    let edited = tg
        .edit_message_text(&chunk.chat_id, entry.progress_message_id, &body)
        .await;
    entry.last_edit = Some(now);
    if chunk.done {
        streams.remove(&chunk.chat_id);
    }
    if let Err(e) = edited {
        tracing::debug!(error = %e, "stream edit failed (non-fatal)");
    }
    Ok(())
}
