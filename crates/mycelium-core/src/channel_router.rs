//! Channel router: normalises per-channel raw events to `mycelium.channel.in`
//! and demuxes admin slash commands.
//!
//! Native rewrite of `components/channel-router/src/lib.rs`. Cross-component
//! WIT calls (`mycelium:agent/agent-registry`, `mycelium:conversation`)
//! collapse into direct method calls on the in-proc `AgentRegistry` /
//! `ConversationStore`. The wash workarounds drop out: no `seen_update`
//! marker-CAS, no deterministic `tg-<update_id>` task ids, no inline
//! Telegram fallback for `step.result` (executor owns that now).

use std::sync::Arc;

use anyhow::{Context, Result};
use async_nats::jetstream::kv::Store;
use chrono::SecondsFormat;
use futures_util::StreamExt;
use mycelium_types::MessageRole;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::{
    agent_registry::AgentRegistry,
    conversation_store::ConversationStore,
    session::SessionStore,
    state::AppState,
    telegram::TgClient,
};

const TELEGRAM_RAW: &str = "mycelium.channel.telegram.raw";
const CHANNEL_IN: &str = "mycelium.channel.in";
const TASK_SUBMIT: &str = "mycelium.task.submit";
const PENDING_BUCKET: &str = "mycelium-channel-pending";

pub async fn run(
    state: AppState,
    registry: AgentRegistry,
    convs: ConversationStore,
    sessions: SessionStore,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    let pending = state
        .js
        .get_key_value(PENDING_BUCKET)
        .await
        .with_context(|| format!("open {PENDING_BUCKET}"))?;

    let ctx = Arc::new(RouterCtx {
        state: state.clone(),
        registry,
        convs,
        sessions,
        pending,
        telegram: if state.config.telegram_bot_token.is_empty() {
            None
        } else {
            Some(TgClient::new(
                state.config.telegram_bot_token.clone(),
                state.http.clone(),
            ))
        },
    });

    let mut sub = state
        .nats
        .subscribe(TELEGRAM_RAW.to_string())
        .await
        .with_context(|| format!("subscribe {TELEGRAM_RAW}"))?;
    tracing::info!(subject = TELEGRAM_RAW, "channel_router started");

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                tracing::info!("channel_router shutting down");
                break;
            }
            maybe = sub.next() => {
                let Some(msg) = maybe else { break };
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_telegram_raw(&ctx2, &msg.payload).await {
                        tracing::warn!(error = %e, "channel_router handler failed");
                    }
                });
            }
        }
    }
    Ok(())
}

struct RouterCtx {
    state: AppState,
    registry: AgentRegistry,
    convs: ConversationStore,
    sessions: SessionStore,
    pending: Store,
    telegram: Option<TgClient>,
}

#[derive(Serialize)]
struct CanonicalMessage<'a> {
    channel: &'a str,
    channel_msg_id: String,
    sender_id: String,
    text: &'a str,
    raw_json: &'a str,
    session_id: String,
}

async fn handle_telegram_raw(ctx: &RouterCtx, body: &[u8]) -> Result<()> {
    let update: Value = serde_json::from_slice(body).context("decode telegram update")?;

    let Some(message) = update.get("message") else {
        return Ok(());
    };
    let chat_id = message
        .pointer("/chat/id")
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())
        .unwrap_or_default();
    let message_id = message
        .get("message_id")
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())
        .unwrap_or_default();
    let text = message.get("text").and_then(|v| v.as_str()).unwrap_or("");

    // Slash commands short-circuit; they don't dispatch the agent loop.
    if try_slash_command(ctx, &chat_id, text).await? {
        return Ok(());
    }
    if try_agent_create(ctx, text).await? {
        return Ok(());
    }

    if text.is_empty() {
        // Still emit canonical for other adapters even if no agent loop runs.
        emit_canonical(ctx, "telegram", &message_id, &chat_id, text, &update.to_string(), "").await?;
        return Ok(());
    }

    let Some(agent_id) = pick_default_agent_for_chat(ctx, &chat_id).await? else {
        tracing::warn!("no agent registered; create one with /agent create");
        return Ok(());
    };

    // Attach (or open) the session for this telegram chat + agent.
    let session = ctx.sessions.attach("telegram", &chat_id, &agent_id).await?;
    ctx.sessions.touch(&session.id).await.ok();

    let raw_json = update.to_string();
    emit_canonical(
        ctx,
        "telegram",
        &message_id,
        &chat_id,
        text,
        &raw_json,
        &session.id,
    )
    .await?;

    if let Some(tg) = &ctx.telegram {
        // Best-effort typing indicator.
        let _ = tg.send_chat_action(&chat_id, "typing").await;
    }

    // Stable conv_id per chat — load if exists, else create.
    let conv_id = match load_chat_conv(&ctx.pending, &chat_id).await? {
        Some(id) => id,
        None => {
            let conv = ctx.convs.create(agent_id.clone(), None).await?;
            save_chat_conv(&ctx.pending, &chat_id, &conv.id).await?;
            conv.id
        }
    };
    ctx.sessions
        .attach_conversation(&session.id, &conv_id)
        .await
        .ok();

    ctx.convs
        .append_message(&conv_id, MessageRole::User, text.to_string(), None)
        .await
        .context("append user message")?;

    let task_id = uuid::Uuid::new_v4().to_string();
    let task = json!({
        "id": task_id,
        "conversation_id": conv_id,
        "agent_id": agent_id,
        "input": text,
        "created_at": now_iso(),
        "session_id": session.id,
    });
    save_pending_full(&ctx.pending, &task_id, "telegram", &chat_id, &conv_id, &message_id).await?;
    let task_bytes = serde_json::to_vec(&task)?;
    ctx.state
        .nats
        .publish(TASK_SUBMIT.to_string(), task_bytes.into())
        .await?;
    Ok(())
}

async fn emit_canonical(
    ctx: &RouterCtx,
    channel: &str,
    channel_msg_id: &str,
    sender_id: &str,
    text: &str,
    raw_json: &str,
    session_id: &str,
) -> Result<()> {
    let canonical = CanonicalMessage {
        channel,
        channel_msg_id: channel_msg_id.to_string(),
        sender_id: sender_id.to_string(),
        text,
        raw_json,
        session_id: session_id.to_string(),
    };
    let bytes = serde_json::to_vec(&canonical)?;
    ctx.state
        .nats
        .publish(CHANNEL_IN.to_string(), bytes.into())
        .await?;
    Ok(())
}

async fn pick_default_agent_for_chat(ctx: &RouterCtx, chat_id: &str) -> Result<Option<String>> {
    if let Some(id) = load_chat_agent(&ctx.pending, chat_id).await? {
        if !id.is_empty() {
            return Ok(Some(id));
        }
    }
    let default = &ctx.state.config.default_agent_id;
    if !default.is_empty() && default != "auto" {
        return Ok(Some(default.clone()));
    }
    ctx.registry.first_agent_id().await
}

async fn load_chat_agent(pending: &Store, chat_id: &str) -> Result<Option<String>> {
    let Some(bytes) = pending.get(format!("default-agent/{chat_id}")).await? else {
        return Ok(None);
    };
    Ok(Some(String::from_utf8(bytes.to_vec())?))
}

async fn save_chat_agent(pending: &Store, chat_id: &str, agent_id: &str) -> Result<()> {
    pending
        .put(
            format!("default-agent/{chat_id}"),
            agent_id.as_bytes().to_vec().into(),
        )
        .await?;
    Ok(())
}

async fn load_chat_conv(pending: &Store, chat_id: &str) -> Result<Option<String>> {
    let Some(bytes) = pending.get(format!("conv/{chat_id}")).await? else {
        return Ok(None);
    };
    Ok(Some(String::from_utf8(bytes.to_vec())?))
}

async fn save_chat_conv(pending: &Store, chat_id: &str, conv_id: &str) -> Result<()> {
    pending
        .put(
            format!("conv/{chat_id}"),
            conv_id.as_bytes().to_vec().into(),
        )
        .await?;
    Ok(())
}

async fn clear_chat_conv(pending: &Store, chat_id: &str) -> Result<()> {
    pending.delete(format!("conv/{chat_id}")).await.ok();
    Ok(())
}

async fn save_pending_full(
    pending: &Store,
    task_id: &str,
    channel: &str,
    chat_id: &str,
    conversation_id: &str,
    channel_msg_id: &str,
) -> Result<()> {
    let pt = json!({
        "channel": channel,
        "chat_id": chat_id,
        "conversation_id": conversation_id,
        "channel_msg_id": channel_msg_id,
    });
    let bytes = serde_json::to_vec(&pt)?;
    pending.put(task_id, bytes.into()).await?;
    Ok(())
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

// ──────────────────────────── slash commands ────────────────────────────

async fn try_slash_command(ctx: &RouterCtx, chat_id: &str, text: &str) -> Result<bool> {
    if !text.starts_with('/') {
        return Ok(false);
    }
    let Some(tg) = &ctx.telegram else {
        return Ok(false);
    };

    let mut it = text.splitn(2, ' ');
    let cmd = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();

    match cmd {
        "/help" | "/start" => {
            tg.send_message(
                chat_id,
                "commands:\n\
                /help — this menu\n\
                /agents — list agents\n\
                /agent set <id> — switch active agent for this chat\n\
                /agent create <id> <model> <prompt> — register a new agent\n\
                /reset — clear conversation history\n\
                /memory — show last 10 messages\n\
                /tools — list registered skills\n\
                /quota — current LLM RPM / RPD",
            )
            .await
            .ok();
            Ok(true)
        }
        "/agents" => {
            let active = load_chat_agent(&ctx.pending, chat_id)
                .await?
                .unwrap_or_else(|| "(none)".to_string());
            let list = match ctx.registry.list_agents().await {
                Ok(v) if v.is_empty() => {
                    "no agents yet — use /agent create <id> <model> <prompt>".to_string()
                }
                Ok(v) => v
                    .iter()
                    .map(|a| {
                        let marker = if a.id == active { "* " } else { "  " };
                        format!("{marker}{} ({})", a.id, a.model)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(e) => format!("list_agents failed: {e}"),
            };
            tg.send_message(chat_id, &list).await.ok();
            Ok(true)
        }
        "/agent" => {
            let mut sub = rest.splitn(2, ' ');
            match sub.next().unwrap_or("") {
                "set" => {
                    let id = sub.next().unwrap_or("").trim();
                    if id.is_empty() {
                        tg.send_message(chat_id, "usage: /agent set <id>").await.ok();
                    } else {
                        save_chat_agent(&ctx.pending, chat_id, id).await?;
                        tg.send_message(chat_id, &format!("default agent for this chat → {id}"))
                            .await
                            .ok();
                    }
                    Ok(true)
                }
                "create" => Ok(false), // existing handler picks it up
                _ => {
                    tg.send_message(
                        chat_id,
                        "usage: /agent set <id> | /agent create <id> <model> <prompt>",
                    )
                    .await
                    .ok();
                    Ok(true)
                }
            }
        }
        "/reset" => {
            if let Some(conv_id) = load_chat_conv(&ctx.pending, chat_id).await? {
                ctx.convs.delete(&conv_id).await.ok();
                clear_chat_conv(&ctx.pending, chat_id).await?;
                tg.send_message(chat_id, "conversation cleared.").await.ok();
            } else {
                tg.send_message(chat_id, "nothing to reset.").await.ok();
            }
            Ok(true)
        }
        "/memory" => {
            let body = match load_chat_conv(&ctx.pending, chat_id).await? {
                None => "no conversation yet.".to_string(),
                Some(cid) => match ctx.convs.get_messages(&cid).await {
                    Err(e) => format!("get_messages failed: {e}"),
                    Ok(msgs) => {
                        let tail: Vec<_> = msgs.iter().rev().take(10).collect();
                        if tail.is_empty() {
                            "(empty)".to_string()
                        } else {
                            tail.iter()
                                .rev()
                                .map(|m| {
                                    let role = match m.role {
                                        MessageRole::User => "you",
                                        MessageRole::Assistant => "bot",
                                        MessageRole::System => "sys",
                                        MessageRole::Tool => "tool",
                                    };
                                    let snippet: String = m.content.chars().take(120).collect();
                                    format!("{role}: {snippet}")
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                },
            };
            tg.send_message(chat_id, &body).await.ok();
            Ok(true)
        }
        "/tools" => {
            let body = list_registered_tools(&ctx.state).await;
            tg.send_message(chat_id, &body).await.ok();
            Ok(true)
        }
        "/quota" | "/budget" => {
            let body = format_quota_for_chat(ctx).await;
            tg.send_message(chat_id, &body).await.ok();
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn try_agent_create(ctx: &RouterCtx, text: &str) -> Result<bool> {
    let Some(rest) = text.strip_prefix("/agent create ") else {
        return Ok(false);
    };
    let mut parts = rest.splitn(3, ' ');
    let id = match parts.next() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return Ok(false),
    };
    let model = match parts.next() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return Ok(false),
    };
    let system_prompt = parts.next().unwrap_or("").to_string();
    let cfg = mycelium_types::AgentConfig {
        id: id.clone(),
        name: id,
        system_prompt,
        model,
        tools: vec![],
        max_steps: 4,
    };
    if let Err(e) = ctx.registry.create(cfg).await {
        tracing::warn!(error = %e, "agent_registry::create failed");
    }
    Ok(true)
}

async fn list_registered_tools(state: &AppState) -> String {
    let Ok(kv) = state.js.get_key_value("mycelium-tools").await else {
        return "skill registry unavailable.".to_string();
    };
    let Ok(mut keys) = kv.keys().await else {
        return "skill registry list failed.".to_string();
    };
    let mut out = String::from("registered skills:\n");
    let mut had_any = false;
    while let Some(key) = keys.next().await {
        let Ok(key) = key else { continue };
        had_any = true;
        let desc = kv
            .get(&key)
            .await
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|v| v.get("description").and_then(|d| d.as_str()).map(str::to_string))
            .unwrap_or_default();
        if desc.is_empty() {
            out.push_str(&format!("- {key}\n"));
        } else {
            out.push_str(&format!("- {key} — {desc}\n"));
        }
    }
    if !had_any {
        return "no skills registered.".to_string();
    }
    out
}

async fn format_quota_for_chat(ctx: &RouterCtx) -> String {
    let Ok(bucket) = ctx.state.js.get_key_value("mycelium-task-state").await else {
        return "(no rate state)".to_string();
    };
    let Ok(mut keys) = bucket.keys().await else {
        return "(rate state list failed)".to_string();
    };
    let now = chrono::Utc::now().timestamp();
    let minute = (now / 60) as u64;
    let day = (now / 86_400) as u64;
    let mut rpm_used = 0u64;
    let mut rpd_used = 0u64;
    while let Some(key) = keys.next().await {
        let Ok(key) = key else { continue };
        let v = bucket.get(&key).await.ok().flatten();
        if key.contains(&format!("/min/{minute}")) {
            if let Some(b) = v.as_ref() {
                if let Ok(s) = std::str::from_utf8(b) {
                    rpm_used += s.parse::<u64>().unwrap_or(0);
                }
            }
        }
        if key.contains(&format!("/day/{day}")) {
            if let Some(b) = v {
                if let Ok(s) = std::str::from_utf8(&b) {
                    rpd_used += s.parse::<u64>().unwrap_or(0);
                }
            }
        }
    }
    format!(
        "quota:\n  RPM {rpm_used}/{}\n  RPD {rpd_used}/{}",
        ctx.state.config.llm_rpm, ctx.state.config.llm_rpd,
    )
}
