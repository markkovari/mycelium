//! Thin Telegram Bot API client for WASI Preview 2 components.
//!
//! Wraps `wstd::http` so callers stay clear of `wasi:http/outgoing-handler`
//! boilerplate. Sync API (uses `wstd::runtime::block_on` internally) so it
//! drops cleanly into `wasmcloud:messaging/handler` exports which are sync.
//!
//! ```ignore
//! use mycelium_telegram::TgClient;
//! let client = TgClient::new("123:abc".into());
//! let updates = client.get_updates(0, 25)?;
//! client.send_message("42", "hello")?;
//! ```

use serde::{Deserialize, Serialize};
use wstd::http::{Body, BodyExt, Client, Method, Request};
use wstd::runtime::block_on;

/// One Telegram update. Subset of the Bot API schema; raw passthrough kept
/// elsewhere if downstream consumers want it.
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
struct TgResponse<T> {
    ok: bool,
    #[serde(default)]
    result: Option<T>,
    #[serde(default)]
    description: Option<String>,
}

/// Bot client. Cheap to construct per call.
#[derive(Debug, Clone)]
pub struct TgClient {
    token: String,
}

impl TgClient {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    /// Long-poll `getUpdates`. Telegram holds the connection up to `timeout_s`
    /// or returns sooner once updates land.
    pub fn get_updates(&self, offset: u64, timeout_s: u32) -> Result<Vec<TgUpdate>, String> {
        let url = format!(
            "https://api.telegram.org/bot{}/getUpdates?offset={offset}&timeout={timeout_s}",
            self.token
        );
        let bytes = http_get(&url)?;
        let parsed: TgResponse<Vec<TgUpdate>> =
            serde_json::from_slice(&bytes).map_err(|e| format!("decode: {e}"))?;
        if !parsed.ok {
            return Err(parsed
                .description
                .unwrap_or_else(|| "telegram returned ok=false".to_string()));
        }
        Ok(parsed.result.unwrap_or_default())
    }

    /// POST `sendMessage`.
    pub fn send_message(&self, chat_id: &str, text: &str) -> Result<(), String> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        let body = serde_json::json!({"chat_id": chat_id, "text": text});
        let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let resp = http_post_json(&url, body_bytes)?;
        let parsed: TgResponse<serde_json::Value> =
            serde_json::from_slice(&resp).map_err(|e| format!("decode: {e}"))?;
        if !parsed.ok {
            return Err(parsed
                .description
                .unwrap_or_else(|| "telegram returned ok=false".to_string()));
        }
        Ok(())
    }

    /// POST `setMessageReaction`. Replaces any existing reactions from this bot.
    /// `emoji` must be one of the values Telegram allows (👍 👎 ❤ 🔥 🥰 👀 …).
    pub fn set_reaction(&self, chat_id: i64, message_id: i64, emoji: &str) -> Result<(), String> {
        let url = format!(
            "https://api.telegram.org/bot{}/setMessageReaction",
            self.token
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{"type": "emoji", "emoji": emoji}],
            "is_big": false,
        });
        let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        let resp = http_post_json(&url, body_bytes)?;
        let parsed: TgResponse<serde_json::Value> =
            serde_json::from_slice(&resp).map_err(|e| format!("decode: {e}"))?;
        if !parsed.ok {
            return Err(parsed
                .description
                .unwrap_or_else(|| "telegram returned ok=false".to_string()));
        }
        Ok(())
    }
}

fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let url = url.to_string();
    block_on(async move {
        let client = Client::new();
        let req = Request::builder()
            .method(Method::GET)
            .uri(url.as_str())
            .body(Body::empty())
            .map_err(|e| format!("build req: {e}"))?;
        let resp = client.send(req).await.map_err(|e| format!("send: {e}"))?;
        let collected = resp
            .into_body()
            .into_boxed_body()
            .collect()
            .await
            .map_err(|e| format!("recv: {e}"))?;
        Ok::<_, String>(collected.to_bytes().to_vec())
    })
}

fn http_post_json(url: &str, body_bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    let url = url.to_string();
    block_on(async move {
        let client = Client::new();
        let req = Request::builder()
            .method(Method::POST)
            .uri(url.as_str())
            .header("content-type", "application/json")
            .body(Body::from(body_bytes))
            .map_err(|e| format!("build req: {e}"))?;
        let resp = client.send(req).await.map_err(|e| format!("send: {e}"))?;
        let collected = resp
            .into_body()
            .into_boxed_body()
            .collect()
            .await
            .map_err(|e| format!("recv: {e}"))?;
        Ok::<_, String>(collected.to_bytes().to_vec())
    })
}
