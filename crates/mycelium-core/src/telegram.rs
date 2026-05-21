//! Native Telegram Bot API client.
//!
//! Replaces the wasi:http-based `mycelium-telegram` crate. async + reqwest.
//! Methods cover everything the wasm pipeline used: getUpdates, sendMessage
//! (with returning message_id), editMessageText, sendChatAction,
//! setMessageReaction, setMyShortDescription.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

const BASE: &str = "https://api.telegram.org";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TgUpdate {
    pub update_id: u64,
    #[serde(default)]
    pub message: Option<TgMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TgMessage {
    pub message_id: i64,
    pub chat: TgChat,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TgChat {
    pub id: i64,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
struct TgResponse<T> {
    ok: bool,
    #[serde(default)]
    result: Option<T>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    error_code: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SentMessage {
    pub message_id: i64,
}

#[derive(Clone)]
pub struct TgClient {
    token: String,
    http: reqwest::Client,
}

impl TgClient {
    pub fn new(token: String, http: reqwest::Client) -> Self {
        Self { token, http }
    }

    pub fn token_is_set(&self) -> bool {
        !self.token.is_empty()
    }

    fn url(&self, method: &str) -> String {
        format!("{BASE}/bot{}/{method}", self.token)
    }

    pub async fn get_updates(&self, offset: u64, timeout_s: u32) -> Result<Vec<TgUpdate>> {
        let resp = self
            .http
            .get(self.url("getUpdates"))
            .query(&[("offset", offset.to_string()), ("timeout", timeout_s.to_string())])
            // Telegram long-poll holds the connection for `timeout_s`. Make
            // sure the underlying request timeout is generous enough to
            // outlast that, with a small buffer.
            .timeout(std::time::Duration::from_secs(u64::from(timeout_s) + 10))
            .send()
            .await
            .context("getUpdates send")?;
        let body = resp.bytes().await.context("getUpdates body")?;
        let parsed: TgResponse<Vec<TgUpdate>> =
            serde_json::from_slice(&body).with_context(|| {
                format!(
                    "getUpdates decode: {}",
                    String::from_utf8_lossy(&body[..body.len().min(400)])
                )
            })?;
        if !parsed.ok {
            return Err(anyhow!(
                "telegram: {} ({})",
                parsed.description.unwrap_or_default(),
                parsed.error_code.unwrap_or(0)
            ));
        }
        Ok(parsed.result.unwrap_or_default())
    }

    pub async fn send_message(&self, chat_id: &str, text: &str) -> Result<SentMessage> {
        let body = serde_json::json!({"chat_id": chat_id, "text": text});
        self.call_returning::<SentMessage>("sendMessage", body).await
    }

    pub async fn send_message_markdown(
        &self,
        chat_id: &str,
        text: &str,
    ) -> Result<SentMessage> {
        let body =
            serde_json::json!({"chat_id": chat_id, "text": text, "parse_mode": "Markdown"});
        self.call_returning::<SentMessage>("sendMessage", body).await
    }

    pub async fn edit_message_text(
        &self,
        chat_id: &str,
        message_id: i64,
        text: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "parse_mode": "Markdown",
        });
        self.call_unit("editMessageText", body).await
    }

    pub async fn send_chat_action(&self, chat_id: &str, action: &str) -> Result<()> {
        let body = serde_json::json!({"chat_id": chat_id, "action": action});
        self.call_unit("sendChatAction", body).await
    }

    pub async fn set_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{"type": "emoji", "emoji": emoji}],
            "is_big": false,
        });
        self.call_unit("setMessageReaction", body).await
    }

    pub async fn set_my_short_description(&self, description: &str) -> Result<()> {
        let body = serde_json::json!({"short_description": description});
        self.call_unit("setMyShortDescription", body).await
    }

    /// POST a JSON body, expect `{"ok": true, "result": T}`.
    async fn call_returning<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        let resp = self
            .http
            .post(self.url(method))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("{method} send"))?;
        let bytes = resp.bytes().await.with_context(|| format!("{method} body"))?;
        let parsed: TgResponse<T> = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "{method} decode: {}",
                String::from_utf8_lossy(&bytes[..bytes.len().min(400)])
            )
        })?;
        if !parsed.ok {
            return Err(anyhow!(
                "telegram {method}: {} ({})",
                parsed.description.unwrap_or_default(),
                parsed.error_code.unwrap_or(0)
            ));
        }
        parsed
            .result
            .ok_or_else(|| anyhow!("telegram {method}: ok=true but no result"))
    }

    /// POST a JSON body where we don't care about the result payload.
    async fn call_unit(&self, method: &str, body: serde_json::Value) -> Result<()> {
        let resp = self
            .http
            .post(self.url(method))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("{method} send"))?;
        let bytes = resp.bytes().await.with_context(|| format!("{method} body"))?;
        let parsed: TgResponse<serde_json::Value> = serde_json::from_slice(&bytes)
            .with_context(|| {
                format!(
                    "{method} decode: {}",
                    String::from_utf8_lossy(&bytes[..bytes.len().min(400)])
                )
            })?;
        if !parsed.ok {
            return Err(anyhow!(
                "telegram {method}: {} ({})",
                parsed.description.unwrap_or_default(),
                parsed.error_code.unwrap_or(0)
            ));
        }
        Ok(())
    }
}
