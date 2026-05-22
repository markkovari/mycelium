//! Session persistence + lookup.
//!
//! A `Session` is the cross-task abstraction OpenClaw exposes via
//! `session_start` / `session_end` hooks: it spans many user turns on the
//! same channel and owns the per-user state that previously lived
//! implicitly in `chat_id`. One Session pins one Conversation; many Tasks
//! can attach to one Session.
//!
//! Layout in `mycelium-sessions` KV bucket:
//!   session/{session_id}                 → SessionRecord JSON
//!   index/{channel}/{channel_session_key} → session_id of the active
//!                                            session on that channel key

use anyhow::{Context, Result};
use async_nats::jetstream::{kv::Store, Context as JsContext};
use chrono::SecondsFormat;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BUCKET: &str = "mycelium-sessions";
const IDLE_CLOSE_SECS: i64 = 24 * 60 * 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub channel: String,
    pub channel_session_key: String,
    pub agent_id: String,
    #[serde(default)]
    pub conversation_id: String,
    pub started_at: String,
    pub last_activity_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Clone)]
pub struct SessionStore {
    kv: Store,
}

impl SessionStore {
    pub async fn open(js: &JsContext) -> Result<Self> {
        let kv = js
            .get_key_value(BUCKET)
            .await
            .with_context(|| format!("open {BUCKET}"))?;
        Ok(Self { kv })
    }

    /// Look up the active session for a `(channel, channel_key)` pair.
    /// If none exists (or the prior one was closed), create a new one.
    pub async fn attach(
        &self,
        channel: &str,
        channel_session_key: &str,
        agent_id: &str,
    ) -> Result<SessionRecord> {
        if let Some(existing) = self.active_for(channel, channel_session_key).await? {
            return Ok(existing);
        }
        let now = now_iso();
        let rec = SessionRecord {
            id: uuid::Uuid::now_v7().to_string(),
            channel: channel.to_string(),
            channel_session_key: channel_session_key.to_string(),
            agent_id: agent_id.to_string(),
            conversation_id: String::new(),
            started_at: now.clone(),
            last_activity_at: now,
            ended_at: None,
            metadata: Value::Null,
        };
        self.save(&rec).await?;
        self.kv
            .put(
                index_key(channel, channel_session_key),
                rec.id.as_bytes().to_vec().into(),
            )
            .await?;
        Ok(rec)
    }

    pub async fn touch(&self, session_id: &str) -> Result<()> {
        let Some(mut rec) = self.get(session_id).await? else {
            return Ok(());
        };
        rec.last_activity_at = now_iso();
        self.save(&rec).await
    }

    pub async fn attach_conversation(&self, session_id: &str, conv_id: &str) -> Result<()> {
        let Some(mut rec) = self.get(session_id).await? else {
            return Ok(());
        };
        if rec.conversation_id == conv_id {
            return Ok(());
        }
        rec.conversation_id = conv_id.to_string();
        rec.last_activity_at = now_iso();
        self.save(&rec).await
    }

    pub async fn close(&self, session_id: &str) -> Result<Option<SessionRecord>> {
        let Some(mut rec) = self.get(session_id).await? else {
            return Ok(None);
        };
        if rec.ended_at.is_some() {
            return Ok(Some(rec));
        }
        rec.ended_at = Some(now_iso());
        self.save(&rec).await?;
        // Drop the secondary index so the next message on this channel-key
        // opens a fresh session.
        self.kv
            .delete(index_key(&rec.channel, &rec.channel_session_key))
            .await
            .ok();
        Ok(Some(rec))
    }

    pub async fn get(&self, session_id: &str) -> Result<Option<SessionRecord>> {
        let key = format!("session/{session_id}");
        let Some(bytes) = self.kv.get(&key).await? else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    pub async fn active_for(
        &self,
        channel: &str,
        channel_session_key: &str,
    ) -> Result<Option<SessionRecord>> {
        let Some(bytes) = self.kv.get(index_key(channel, channel_session_key)).await? else {
            return Ok(None);
        };
        let session_id = String::from_utf8(bytes.to_vec())?;
        let rec = self.get(&session_id).await?;
        Ok(rec.filter(|r| r.ended_at.is_none()))
    }

    pub async fn list_active(&self) -> Result<Vec<SessionRecord>> {
        let mut out = Vec::new();
        let mut keys = self.kv.keys().await?;
        while let Some(key) = keys.next().await {
            let Ok(key) = key else { continue };
            if !key.starts_with("session/") {
                continue;
            }
            let Some(bytes) = self.kv.get(&key).await.ok().flatten() else {
                continue;
            };
            let Ok(rec) = serde_json::from_slice::<SessionRecord>(&bytes) else {
                continue;
            };
            if rec.ended_at.is_none() {
                out.push(rec);
            }
        }
        Ok(out)
    }

    /// Close any session whose `last_activity_at` is older than 24h. Returns
    /// the list of session ids that got closed.
    pub async fn sweep_idle(&self) -> Result<Vec<String>> {
        let now = chrono::Utc::now();
        let active = self.list_active().await?;
        let mut closed = Vec::new();
        for rec in active {
            let parsed = chrono::DateTime::parse_from_rfc3339(&rec.last_activity_at).ok();
            let idle_secs = parsed
                .map(|t| (now - t.with_timezone(&chrono::Utc)).num_seconds())
                .unwrap_or(0);
            if idle_secs > IDLE_CLOSE_SECS && self.close(&rec.id).await?.is_some() {
                closed.push(rec.id);
            }
        }
        Ok(closed)
    }

    async fn save(&self, rec: &SessionRecord) -> Result<()> {
        let bytes = serde_json::to_vec(rec)?;
        self.kv
            .put(format!("session/{}", rec.id), bytes.into())
            .await?;
        Ok(())
    }
}

fn index_key(channel: &str, channel_session_key: &str) -> String {
    format!("index/{channel}/{channel_session_key}")
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
