//! Skill registry — manifests live in the `mycelium-skills` JetStream KV
//! bucket as JSON. Operators register skills with
//! `mycelium skill register <manifest.json>`, the CLI puts the JSON here,
//! the runner reads it on every tool call.
//!
//! Cached in memory with a coarse TTL so repeated calls to the same skill
//! don't hammer NATS.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::RwLock;

use crate::manifest::SkillManifest;

const BUCKET: &str = "mycelium-skills";
const CACHE_TTL: Duration = Duration::from_secs(30);

pub struct SkillRegistry {
    kv: async_nats::jetstream::kv::Store,
    cache: RwLock<Option<(Instant, Vec<SkillManifest>)>>,
}

impl SkillRegistry {
    pub async fn open(js: &async_nats::jetstream::Context) -> Result<Self> {
        // Idempotent create: try get; if missing, create a small history
        // bucket.
        let kv = match js.get_key_value(BUCKET).await {
            Ok(s) => s,
            Err(_) => js
                .create_key_value(async_nats::jetstream::kv::Config {
                    bucket: BUCKET.to_string(),
                    description: "mycelium skill manifests".into(),
                    history: 5,
                    ..Default::default()
                })
                .await
                .with_context(|| format!("create KV bucket {BUCKET}"))?,
        };
        Ok(Self {
            kv,
            cache: RwLock::new(None),
        })
    }

    pub async fn get(&self, name: &str) -> Result<Option<SkillManifest>> {
        if let Some(m) = self.cached_lookup(name).await {
            return Ok(Some(m));
        }
        let manifests = self.refresh().await?;
        Ok(manifests.into_iter().find(|m| m.name == name))
    }

    pub async fn list(&self) -> Result<Vec<SkillManifest>> {
        self.refresh().await
    }

    async fn cached_lookup(&self, name: &str) -> Option<SkillManifest> {
        let cache = self.cache.read().await;
        let (when, ref ms) = cache.as_ref()?;
        if when.elapsed() > CACHE_TTL {
            return None;
        }
        ms.iter().find(|m| m.name == name).cloned()
    }

    async fn refresh(&self) -> Result<Vec<SkillManifest>> {
        use futures_util::StreamExt;
        let mut keys = self.kv.keys().await?;
        let mut out = Vec::new();
        while let Some(key) = keys.next().await {
            let Ok(key) = key else { continue };
            let Some(bytes) = self.kv.get(&key).await? else { continue };
            match serde_json::from_slice::<SkillManifest>(&bytes) {
                Ok(m) => out.push(m),
                Err(e) => tracing::warn!(skill = %key, error = %e, "invalid skill manifest"),
            }
        }
        let mut cache = self.cache.write().await;
        *cache = Some((Instant::now(), out.clone()));
        Ok(out)
    }
}
