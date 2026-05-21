//! Agent memory (KV-backed).
//!
//! Ported from `components/memory-store/src/lib.rs`. Same bucket
//! (`mycelium-memory`) and key scheme (`memory/{agent_id}/{key}`).
//!
//! In the wasm world this component also exported a `mycelium.memory.>`
//! NATS request/reply surface for native clients. We drop that surface in
//! the native rewrite — the in-process call is the canonical path; external
//! clients should go through the gateway HTTP API.

use anyhow::{Context, Result};
use async_nats::jetstream::kv::Store;
use futures_util::StreamExt;

const BUCKET: &str = "mycelium-memory";

fn key(agent_id: &str, key: &str) -> String {
    format!("memory/{agent_id}/{key}")
}

#[derive(Clone)]
pub struct MemoryStore {
    kv: Store,
}

impl MemoryStore {
    pub async fn open(js: &async_nats::jetstream::Context) -> Result<Self> {
        let kv = js
            .get_key_value(BUCKET)
            .await
            .with_context(|| format!("open KV bucket {BUCKET}"))?;
        Ok(Self { kv })
    }

    pub async fn set(&self, agent_id: &str, k: &str, value: &str) -> Result<()> {
        self.kv
            .put(key(agent_id, k), value.as_bytes().to_vec().into())
            .await?;
        Ok(())
    }

    pub async fn get(&self, agent_id: &str, k: &str) -> Result<Option<String>> {
        let Some(bytes) = self.kv.get(key(agent_id, k)).await? else {
            return Ok(None);
        };
        Ok(Some(String::from_utf8(bytes.to_vec())?))
    }

    pub async fn delete(&self, agent_id: &str, k: &str) -> Result<()> {
        self.kv.delete(key(agent_id, k)).await?;
        Ok(())
    }

    pub async fn list_keys(&self, agent_id: &str) -> Result<Vec<String>> {
        let prefix = format!("memory/{agent_id}/");
        let mut keys = self.kv.keys().await?;
        let mut out = Vec::new();
        while let Some(key) = keys.next().await {
            let key = key?;
            if let Some(suffix) = key.strip_prefix(&prefix) {
                out.push(suffix.to_string());
            }
        }
        Ok(out)
    }

    pub async fn set_many(&self, agent_id: &str, entries: Vec<(String, String)>) -> Result<()> {
        for (k, v) in entries {
            self.set(agent_id, &k, &v).await?;
        }
        Ok(())
    }
}
