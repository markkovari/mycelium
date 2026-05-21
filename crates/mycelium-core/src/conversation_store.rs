//! Conversation + message persistence.
//!
//! Ported from `components/conversation-store/src/lib.rs`. Same KV bucket
//! names + key schemes so existing data on the Pi keeps working unchanged.
//!
//! Buckets:
//! * `mycelium-conversations` — `conv/{conversation_id}` → `ConvJson`
//! * `mycelium-messages`      — `msg/{conv_id}/{ns_ts:020}_{msg_id}` → `MsgJson`

use anyhow::{Context, Result};
use async_nats::jetstream::kv::Store;
use chrono::SecondsFormat;
use futures_util::StreamExt;
use mycelium_types::{Conversation, Message, MessageRole};
use serde::{Deserialize, Serialize};

const CONV_BUCKET: &str = "mycelium-conversations";
const MSG_BUCKET: &str = "mycelium-messages";

#[derive(Clone)]
pub struct ConversationStore {
    convs: Store,
    msgs: Store,
}

impl ConversationStore {
    pub async fn open(js: &async_nats::jetstream::Context) -> Result<Self> {
        let convs = js
            .get_key_value(CONV_BUCKET)
            .await
            .with_context(|| format!("open KV bucket {CONV_BUCKET}"))?;
        let msgs = js
            .get_key_value(MSG_BUCKET)
            .await
            .with_context(|| format!("open KV bucket {MSG_BUCKET}"))?;
        Ok(Self { convs, msgs })
    }

    pub async fn create(&self, agent_id: String, title: Option<String>) -> Result<Conversation> {
        let id = uuid_v7();
        let now = now_iso();
        let conv = Conversation {
            id: id.clone(),
            agent_id,
            title,
            created_at: now.clone(),
            updated_at: now,
        };
        let json = serde_json::to_vec(&ConvJson::from(&conv))?;
        self.convs
            .put(format!("conv/{id}"), json.into())
            .await
            .with_context(|| format!("put conv/{id}"))?;
        Ok(conv)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Conversation>> {
        let Some(bytes) = self
            .convs
            .get(format!("conv/{id}"))
            .await
            .with_context(|| format!("get conv/{id}"))?
        else {
            return Ok(None);
        };
        let j: ConvJson = serde_json::from_slice(&bytes)?;
        Ok(Some(j.into()))
    }

    pub async fn list_for_agent(&self, agent_id: &str) -> Result<Vec<Conversation>> {
        let mut keys = self.convs.keys().await?;
        let mut out = Vec::new();
        while let Some(key) = keys.next().await {
            let key = key?;
            if !key.starts_with("conv/") {
                continue;
            }
            if let Some(bytes) = self.convs.get(&key).await? {
                if let Ok(j) = serde_json::from_slice::<ConvJson>(&bytes) {
                    if j.agent_id == agent_id {
                        out.push(j.into());
                    }
                }
            }
        }
        Ok(out)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.convs.delete(format!("conv/{id}")).await.ok();

        let prefix = format!("msg/{id}/");
        let mut keys = self.msgs.keys().await?;
        while let Some(key) = keys.next().await {
            let key = key?;
            if key.starts_with(&prefix) {
                self.msgs.delete(&key).await.ok();
            }
        }
        Ok(())
    }

    pub async fn append_message(
        &self,
        conversation_id: &str,
        role: MessageRole,
        content: String,
        tool_use_id: Option<String>,
    ) -> Result<Message> {
        let msg_id = uuid_v7();
        let now = now_iso();
        let stamp = chrono::Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000) as u64;
        let msg = Message {
            id: msg_id.clone(),
            role,
            content,
            tool_use_id,
            created_at: now.clone(),
        };
        let json = serde_json::to_vec(&MsgJson::from(&msg))?;
        self.msgs
            .put(
                format!("msg/{conversation_id}/{stamp:020}_{msg_id}"),
                json.into(),
            )
            .await?;

        if let Ok(Some(bytes)) = self.convs.get(format!("conv/{conversation_id}")).await {
            if let Ok(mut j) = serde_json::from_slice::<ConvJson>(&bytes) {
                j.updated_at = now;
                if let Ok(updated) = serde_json::to_vec(&j) {
                    let _ = self
                        .convs
                        .put(format!("conv/{conversation_id}"), updated.into())
                        .await;
                }
            }
        }
        Ok(msg)
    }

    pub async fn get_messages(&self, conversation_id: &str) -> Result<Vec<Message>> {
        let prefix = format!("msg/{conversation_id}/");
        let mut keys_stream = self.msgs.keys().await?;
        let mut keys: Vec<String> = Vec::new();
        while let Some(key) = keys_stream.next().await {
            let key = key?;
            if key.starts_with(&prefix) {
                keys.push(key);
            }
        }
        keys.sort();
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(bytes) = self.msgs.get(&key).await? {
                if let Ok(j) = serde_json::from_slice::<MsgJson>(&bytes) {
                    out.push(j.into());
                }
            }
        }
        Ok(out)
    }

    pub async fn get_messages_after(
        &self,
        conversation_id: &str,
        after_id: &str,
    ) -> Result<Vec<Message>> {
        let all = self.get_messages(conversation_id).await?;
        let mut found = false;
        let mut out = Vec::new();
        for m in all {
            if found {
                out.push(m);
            } else if m.id == after_id {
                found = true;
            }
        }
        Ok(out)
    }
}

// On-disk JSON shapes. Kept structurally identical to
// `components/conversation-store/src/lib.rs` so we read existing data
// without migration.

#[derive(Serialize, Deserialize)]
struct ConvJson {
    id: String,
    agent_id: String,
    title: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<&Conversation> for ConvJson {
    fn from(c: &Conversation) -> Self {
        Self {
            id: c.id.clone(),
            agent_id: c.agent_id.clone(),
            title: c.title.clone(),
            created_at: c.created_at.clone(),
            updated_at: c.updated_at.clone(),
        }
    }
}

impl From<ConvJson> for Conversation {
    fn from(j: ConvJson) -> Self {
        Self {
            id: j.id,
            agent_id: j.agent_id,
            title: j.title,
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct MsgJson {
    id: String,
    role: String,
    content: String,
    tool_use_id: Option<String>,
    created_at: String,
}

fn role_to_str(r: &MessageRole) -> &'static str {
    match r {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

fn role_from_str(s: &str) -> MessageRole {
    match s {
        "system" => MessageRole::System,
        "assistant" => MessageRole::Assistant,
        "tool" => MessageRole::Tool,
        _ => MessageRole::User,
    }
}

impl From<&Message> for MsgJson {
    fn from(m: &Message) -> Self {
        Self {
            id: m.id.clone(),
            role: role_to_str(&m.role).to_string(),
            content: m.content.clone(),
            tool_use_id: m.tool_use_id.clone(),
            created_at: m.created_at.clone(),
        }
    }
}

impl From<MsgJson> for Message {
    fn from(j: MsgJson) -> Self {
        Self {
            id: j.id,
            role: role_from_str(&j.role),
            content: j.content,
            tool_use_id: j.tool_use_id,
            created_at: j.created_at,
        }
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn uuid_v7() -> String {
    uuid::Uuid::now_v7().to_string()
}
