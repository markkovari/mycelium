//! Conversation compaction.
//!
//! When a conversation exceeds the configured threshold (default: 40
//! messages OR 8000-char combined content), the older head is summarised
//! by a cheap LLM call. The history is rewritten to
//!
//!   [ System("context summary: …"),  …last RESERVE_RECENT messages ]
//!
//! so subsequent LLM calls see compact context. Hook events
//! `before-compaction` / `after-compaction` fire around the summary call;
//! the `before-` hook may set `{block: true}` to skip the pass.
//!
//! Failures are non-fatal — callers receive the original history.

use anyhow::Result;
use mycelium_types::{Message, MessageRole};
use serde_json::{json, Value};

use crate::{conversation_store::ConversationStore, hooks, state::AppState};

/// Default trigger thresholds. Per-agent values in `StoredAgentConfig`
/// override the message-count side.
pub const DEFAULT_MSG_THRESHOLD: usize = 40;
pub const DEFAULT_CHAR_THRESHOLD: usize = 8_000;
pub const RESERVE_RECENT: usize = 10;

/// Inspect `history` and compact it in place (also rewriting the KV) if
/// it has grown past the thresholds. Returns the (possibly mutated) list.
#[allow(clippy::too_many_arguments)]
pub async fn maybe_compact(
    state: &AppState,
    convs: &ConversationStore,
    conversation_id: &str,
    agent_id: &str,
    history: Vec<Message>,
    msg_threshold: usize,
    summary_model: &str,
    api_key: &str,
    endpoint: &str,
) -> Vec<Message> {
    let total_chars: usize = history.iter().map(|m| m.content.chars().count()).sum();
    if history.len() < msg_threshold && total_chars < DEFAULT_CHAR_THRESHOLD {
        return history;
    }
    if history.len() <= RESERVE_RECENT {
        return history;
    }

    let pre = hooks::fire(
        &state.nats,
        "before-compaction",
        json!({
            "conversation_id": conversation_id,
            "agent_id": agent_id,
            "message_count": history.len(),
            "char_count": total_chars,
        }),
    )
    .await;
    if pre.block {
        tracing::info!(reason = ?pre.reason, "before-compaction blocked");
        return history;
    }

    let split = history.len() - RESERVE_RECENT;
    let head = &history[..split];
    let tail = history[split..].to_vec();

    let summary = match summarise(state, head, summary_model, api_key, endpoint).await {
        Ok(s) if !s.trim().is_empty() => s,
        Ok(_) => {
            tracing::warn!("compaction summary came back empty; skipping rewrite");
            return history;
        }
        Err(e) => {
            tracing::warn!(error = %e, "compaction summarisation failed; skipping rewrite");
            return history;
        }
    };

    let summary_msg = Message {
        id: uuid::Uuid::now_v7().to_string(),
        role: MessageRole::System,
        content: format!("Conversation summary so far: {summary}"),
        tool_use_id: None,
        created_at: chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
    };
    let mut new_history = Vec::with_capacity(tail.len() + 1);
    new_history.push(summary_msg);
    new_history.extend(tail);

    if let Err(e) = convs.replace_history(conversation_id, &new_history).await {
        tracing::warn!(error = %e, "replace_history failed; in-memory history still compacted but KV stale");
    }

    let _ = hooks::fire(
        &state.nats,
        "after-compaction",
        json!({
            "conversation_id": conversation_id,
            "agent_id": agent_id,
            "old_count": history.len(),
            "new_count": new_history.len(),
            "summary_chars": new_history[0].content.chars().count(),
        }),
    )
    .await;

    new_history
}

async fn summarise(
    state: &AppState,
    head: &[Message],
    model: &str,
    api_key: &str,
    endpoint: &str,
) -> Result<String> {
    let mut transcript = String::new();
    for m in head {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        };
        transcript.push_str(&format!("{role}: {}\n", m.content));
    }
    let body = json!({
        "model": model,
        "stream": false,
        "messages": [
            {
                "role": "system",
                "content": "You compress prior chat history. Reply with ONE paragraph capturing every concrete fact, identity, decision, open question, and personal preference. Be terse. Do not invent."
            },
            {
                "role": "user",
                "content": format!("Compress this transcript:\n\n{transcript}")
            }
        ]
    });
    let mut req = state.http.post(endpoint).json(&body);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let snippet = resp.text().await.unwrap_or_default();
        anyhow::bail!("HTTP {}: {}", status.as_u16(), snippet);
    }
    let parsed: Value = resp.json().await?;
    let content = parsed
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(content)
}
