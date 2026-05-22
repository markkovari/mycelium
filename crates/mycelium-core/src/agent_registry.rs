//! Per-agent configuration registry.
//!
//! Ported from `components/agent-registry/src/lib.rs`. Same bucket
//! (`mycelium-agent-config`) and same key scheme (`agent/{id}`) so existing
//! agent records keep working.
//!
//! `StoredAgentConfig` extends the public `AgentConfig` with two optional
//! KV-side fields the public API doesn't expose: `endpoint` + `api_key`. They
//! let an operator pin a custom LLM endpoint per agent without baking it into
//! the binary.

use anyhow::{anyhow, Context, Result};
use async_nats::jetstream::kv::Store;
use futures_util::StreamExt;
use mycelium_types::AgentConfig;
use serde::{Deserialize, Serialize};

const BUCKET: &str = "mycelium-agent-config";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredAgentConfig {
    pub id: String,
    pub name: String,
    pub system_prompt: String,
    pub model: String,
    pub tools: Vec<String>,
    pub max_steps: u32,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    /// Trigger compaction when message count exceeds this. Zero or
    /// `None` falls back to the global default. Per-agent only.
    #[serde(default)]
    pub compaction_threshold: Option<u32>,
    /// Optional cheaper model used for summarisation. Falls back to the
    /// agent's main `model` when unset.
    #[serde(default)]
    pub compaction_model: Option<String>,
}

impl From<&AgentConfig> for StoredAgentConfig {
    fn from(c: &AgentConfig) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            system_prompt: c.system_prompt.clone(),
            model: c.model.clone(),
            tools: c.tools.clone(),
            max_steps: c.max_steps,
            endpoint: None,
            api_key: None,
            compaction_threshold: None,
            compaction_model: None,
        }
    }
}

impl From<StoredAgentConfig> for AgentConfig {
    fn from(s: StoredAgentConfig) -> Self {
        Self {
            id: s.id,
            name: s.name,
            system_prompt: s.system_prompt,
            model: s.model,
            tools: s.tools,
            max_steps: s.max_steps,
        }
    }
}

#[derive(Clone)]
pub struct AgentRegistry {
    kv: Store,
}

impl AgentRegistry {
    pub async fn open(js: &async_nats::jetstream::Context) -> Result<Self> {
        let kv = js
            .get_key_value(BUCKET)
            .await
            .with_context(|| format!("open KV bucket {BUCKET}"))?;
        Ok(Self { kv })
    }

    pub async fn create(&self, config: AgentConfig) -> Result<AgentConfig> {
        if config.id.trim().is_empty() {
            return Err(anyhow!("agent id required"));
        }
        let key = format!("agent/{}", config.id);
        if self.kv.get(&key).await?.is_some() {
            return Err(anyhow!("agent {} already exists", config.id));
        }
        let stored = StoredAgentConfig::from(&config);
        let bytes = serde_json::to_vec(&stored)?;
        self.kv.put(&key, bytes.into()).await?;
        Ok(stored.into())
    }

    pub async fn get(&self, id: &str) -> Result<Option<AgentConfig>> {
        Ok(self.get_stored(id).await?.map(Into::into))
    }

    /// Same as `get`, but exposes the extension fields (`endpoint`, `api_key`)
    /// the public `AgentConfig` doesn't carry. The agent module needs this.
    pub async fn get_stored(&self, id: &str) -> Result<Option<StoredAgentConfig>> {
        let key = format!("agent/{id}");
        let Some(bytes) = self.kv.get(&key).await? else {
            return Ok(None);
        };
        let stored: StoredAgentConfig = serde_json::from_slice(&bytes)?;
        Ok(Some(stored))
    }

    pub async fn update(&self, config: AgentConfig) -> Result<AgentConfig> {
        let existing = self
            .get_stored(&config.id)
            .await?
            .ok_or_else(|| anyhow!("agent {} not found", config.id))?;
        let mut stored = StoredAgentConfig::from(&config);
        stored.endpoint = existing.endpoint;
        stored.api_key = existing.api_key;
        let bytes = serde_json::to_vec(&stored)?;
        self.kv
            .put(format!("agent/{}", config.id), bytes.into())
            .await?;
        Ok(stored.into())
    }

    /// Update only the endpoint + api_key extension fields. Public WIT had no
    /// way to do this; operator CLI used to write the KV directly. Expose it
    /// here so the CLI talks to mycelium-core instead.
    pub async fn set_endpoint_and_key(
        &self,
        id: &str,
        endpoint: Option<String>,
        api_key: Option<String>,
    ) -> Result<()> {
        let mut stored = self
            .get_stored(id)
            .await?
            .ok_or_else(|| anyhow!("agent {id} not found"))?;
        stored.endpoint = endpoint;
        stored.api_key = api_key;
        let bytes = serde_json::to_vec(&stored)?;
        self.kv.put(format!("agent/{id}"), bytes.into()).await?;
        Ok(())
    }

    pub async fn set_tools(&self, id: &str, tools: Vec<String>) -> Result<()> {
        let mut stored = self
            .get_stored(id)
            .await?
            .ok_or_else(|| anyhow!("agent {id} not found"))?;
        stored.tools = tools;
        let bytes = serde_json::to_vec(&stored)?;
        self.kv.put(format!("agent/{id}"), bytes.into()).await?;
        Ok(())
    }

    pub async fn set_max_steps(&self, id: &str, max_steps: u32) -> Result<()> {
        let mut stored = self
            .get_stored(id)
            .await?
            .ok_or_else(|| anyhow!("agent {id} not found"))?;
        stored.max_steps = max_steps;
        let bytes = serde_json::to_vec(&stored)?;
        self.kv.put(format!("agent/{id}"), bytes.into()).await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.kv.delete(format!("agent/{id}")).await?;
        Ok(())
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentConfig>> {
        let mut keys = self.kv.keys().await?;
        let mut out = Vec::new();
        while let Some(key) = keys.next().await {
            let key = key?;
            if !key.starts_with("agent/") {
                continue;
            }
            if let Some(bytes) = self.kv.get(&key).await? {
                if let Ok(stored) = serde_json::from_slice::<StoredAgentConfig>(&bytes) {
                    out.push(stored.into());
                }
            }
        }
        Ok(out)
    }

    /// Pick the first agent in the registry. Used by channel-router as the
    /// "auto" fallback when no per-chat default agent is set.
    pub async fn first_agent_id(&self) -> Result<Option<String>> {
        let agents = self.list_agents().await?;
        Ok(agents.into_iter().next().map(|a| a.id))
    }
}
