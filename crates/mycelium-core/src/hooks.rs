//! Hook firing helpers.
//!
//! mycelium-hook-runner sits behind `mycelium.hook.<event-name>` and
//! responds via NATS request-reply. Each callsite in this module fires
//! the event with a tight timeout; the runner echoes the payload back
//! when no hook is registered, so callers can treat hooks as transparent
//! pass-through when the operator hasn't installed any.
//!
//! Reply contract: the runner returns whatever the hook component
//! produced. If the JSON parses as `{ "block": true, "reason": ... }`,
//! the caller MUST honor the block (e.g. skip the tool call). Anything
//! else is the mutated payload to thread forward.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const HOOK_TIMEOUT: Duration = Duration::from_millis(2_000);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookOutcome {
    pub payload: Value,
    pub block: bool,
    pub reason: Option<String>,
}

impl HookOutcome {
    pub fn passthrough(payload: Value) -> Self {
        Self {
            payload,
            block: false,
            reason: None,
        }
    }
}

/// Fire a hook event and return the (possibly mutated) payload.
///
/// Errors and timeouts log a warning and degrade to pass-through (the
/// original payload, no block). This keeps the request path resilient to
/// hook-runner restarts or buggy hook components.
pub async fn fire(nats: &async_nats::Client, event: &str, payload: Value) -> HookOutcome {
    let subject = format!("mycelium.hook.{event}");
    let body = match serde_json::to_vec(&payload) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(event, error = %e, "hook payload serialize failed");
            return HookOutcome::passthrough(payload);
        }
    };
    let req = tokio::time::timeout(HOOK_TIMEOUT, nats.request(subject, body.into())).await;
    let msg = match req {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            tracing::debug!(event, error = %e, "hook request failed; passthrough");
            return HookOutcome::passthrough(payload);
        }
        Err(_) => {
            tracing::debug!(event, "hook request timed out; passthrough");
            return HookOutcome::passthrough(payload);
        }
    };
    let reply_str = match std::str::from_utf8(&msg.payload) {
        Ok(s) if !s.is_empty() => s,
        _ => return HookOutcome::passthrough(payload),
    };
    let parsed: Value = match serde_json::from_str(reply_str) {
        Ok(v) => v,
        Err(_) => return HookOutcome::passthrough(payload),
    };
    let block = parsed
        .get("block")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reason = parsed
        .get("reason")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    HookOutcome {
        payload: parsed,
        block,
        reason,
    }
}
