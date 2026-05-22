//! Event fan-out router + durable journal.
//!
//! Two modules merged into one tokio task: `router` (rules-based subject
//! fan-out from `components/router/src/lib.rs`) + `event-logger` (audit log
//! from `components/event-logger/src/lib.rs`). Both subscribe to
//! `mycelium.event.>`; previously they were separate workloads racing the
//! same subject. Native consolidates them.
//!
//! KV buckets (unchanged):
//! * `mycelium-router-rules`     — `rule/{pattern}` → `RuleJson`
//! * `mycelium-events-journal`   — `events/{subject}/{ns_ts:020}` → raw body

use anyhow::{Context, Result};
use async_nats::jetstream::kv::Store;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::state::AppState;

const RULES_BUCKET: &str = "mycelium-router-rules";
const JOURNAL_BUCKET: &str = "mycelium-events-journal";
const SUBJECT: &str = "mycelium.event.>";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouterRule {
    pub subject_pattern: String,
    pub targets: Vec<String>,
    pub filter_jmespath: Option<String>,
}

pub async fn run(state: AppState, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
    let rules_kv = state
        .js
        .get_key_value(RULES_BUCKET)
        .await
        .with_context(|| format!("open {RULES_BUCKET}"))?;
    let journal_kv = state
        .js
        .get_key_value(JOURNAL_BUCKET)
        .await
        .with_context(|| format!("open {JOURNAL_BUCKET}"))?;

    let mut sub = state
        .nats
        .subscribe(SUBJECT.to_string())
        .await
        .with_context(|| format!("subscribe {SUBJECT}"))?;

    tracing::info!(subject = SUBJECT, "events task started");

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                tracing::info!("events task shutting down");
                break;
            }
            maybe = sub.next() => {
                let Some(msg) = maybe else {
                    tracing::warn!("events subscription closed");
                    break;
                };
                if let Err(e) = handle_event(&state, &rules_kv, &journal_kv, &msg).await {
                    tracing::warn!(error = %e, subject = %msg.subject, "events handler failed");
                }
            }
        }
    }
    Ok(())
}

async fn handle_event(
    state: &AppState,
    rules_kv: &Store,
    journal_kv: &Store,
    msg: &async_nats::Message,
) -> Result<()> {
    // 1. journal
    let stamp = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000) as u64;
    let key = format!("events/{}/{stamp:020}", msg.subject);
    journal_kv.put(key, msg.payload.clone()).await?;

    // 2. fan-out via rules
    let rules = load_rules(rules_kv).await?;
    for rule in &rules {
        if subject_matches(&rule.subject_pattern, &msg.subject) {
            for target in &rule.targets {
                state
                    .nats
                    .publish(target.clone(), msg.payload.clone())
                    .await?;
            }
        }
    }
    Ok(())
}

async fn load_rules(kv: &Store) -> Result<Vec<RouterRule>> {
    let mut keys = kv.keys().await?;
    let mut out = Vec::new();
    while let Some(key) = keys.next().await {
        let key = key?;
        if !key.starts_with("rule/") {
            continue;
        }
        if let Some(bytes) = kv.get(&key).await? {
            if let Ok(r) = serde_json::from_slice::<RouterRule>(&bytes) {
                out.push(r);
            }
        }
    }
    Ok(out)
}

/// NATS subject pattern matcher. `*` = one segment, `>` = one or more tail segments.
fn subject_matches(pattern: &str, subject: &str) -> bool {
    let p: Vec<&str> = pattern.split('.').collect();
    let s: Vec<&str> = subject.split('.').collect();
    let mut i = 0;
    while i < p.len() {
        if p[i] == ">" {
            return i < s.len();
        }
        if i >= s.len() {
            return false;
        }
        if p[i] != "*" && p[i] != s[i] {
            return false;
        }
        i += 1;
    }
    i == s.len()
}

#[cfg(test)]
mod tests {
    use super::subject_matches;

    #[test]
    fn exact() {
        assert!(subject_matches("a.b.c", "a.b.c"));
        assert!(!subject_matches("a.b.c", "a.b"));
        assert!(!subject_matches("a.b.c", "a.b.c.d"));
    }
    #[test]
    fn star() {
        assert!(subject_matches("a.*.c", "a.b.c"));
        assert!(subject_matches("a.*.c", "a.x.c"));
        assert!(!subject_matches("a.*.c", "a.b"));
    }
    #[test]
    fn gt() {
        assert!(subject_matches("a.>", "a.b"));
        assert!(subject_matches("a.>", "a.b.c.d"));
        assert!(!subject_matches("a.>", "a"));
    }
}
