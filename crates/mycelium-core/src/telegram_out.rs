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

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{state::AppState, telegram::TgClient};

const OUT_SUBJECT: &str = "mycelium.channel.telegram.out.>";
const ACTION_SUBJECT: &str = "mycelium.channel.telegram.action.>";

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
    tracing::info!("telegram_out started");

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
