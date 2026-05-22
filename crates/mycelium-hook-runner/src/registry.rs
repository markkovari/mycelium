//! Hook registry — manifests live in the `mycelium-hooks` JetStream KV
//! bucket. Each manifest is keyed by the event name it subscribes to
//! (e.g. `before-tool-call`, `after-tool-call`, `session-start`). Multiple
//! hooks per event are not yet supported — first writer wins.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::RwLock;

use crate::HookManifest;

const BUCKET: &str = "mycelium-hooks";
const CACHE_TTL: Duration = Duration::from_secs(15);

pub struct HookRegistry {
    kv: async_nats::jetstream::kv::Store,
    cache: RwLock<Option<(Instant, Vec<(String, HookManifest)>)>>,
}

impl HookRegistry {
    pub async fn open(js: &async_nats::jetstream::Context) -> Result<Self> {
        let kv = match js.get_key_value(BUCKET).await {
            Ok(s) => s,
            Err(_) => js
                .create_key_value(async_nats::jetstream::kv::Config {
                    bucket: BUCKET.to_string(),
                    description: "mycelium hook manifests".into(),
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

    pub async fn get(&self, event: &str) -> Result<Option<HookManifest>> {
        if let Some(m) = self.cached_lookup(event).await {
            return Ok(Some(m));
        }
        let manifests = self.refresh().await?;
        Ok(manifests.into_iter().find_map(|(ev, m)| if ev == event { Some(m) } else { None }))
    }

    async fn cached_lookup(&self, event: &str) -> Option<HookManifest> {
        let cache = self.cache.read().await;
        let (when, ref ms) = cache.as_ref()?;
        if when.elapsed() > CACHE_TTL {
            return None;
        }
        ms.iter()
            .find(|(ev, _)| ev == event)
            .map(|(_, m)| m.clone())
    }

    async fn refresh(&self) -> Result<Vec<(String, HookManifest)>> {
        use futures_util::StreamExt;
        let mut keys = self.kv.keys().await?;
        let mut out = Vec::new();
        while let Some(key) = keys.next().await {
            let Ok(key) = key else { continue };
            let Some(bytes) = self.kv.get(&key).await? else { continue };
            match serde_json::from_slice::<HookManifest>(&bytes) {
                Ok(m) => out.push((key, m)),
                Err(e) => tracing::warn!(hook = %key, error = %e, "invalid hook manifest"),
            }
        }
        let mut cache = self.cache.write().await;
        *cache = Some((Instant::now(), out.clone()));
        Ok(out)
    }
}
