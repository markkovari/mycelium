// Per-agent configuration registry.
// Exports mycelium:agent/agent-registry.
// KV bucket: mycelium-agent-config, key `agent/{id}`, value StoredAgentConfig JSON.
//
// StoredAgentConfig wraps the canonical AgentConfig with optional endpoint + api_key
// extension fields. The WIT record stays clean; extension fields live KV-side only.
wit_bindgen::generate!({
    path: "wit",
    world: "agent-registry",
    generate_all,
});

use serde::{Deserialize, Serialize};

use exports::mycelium::agent::agent_registry::Guest;
use mycelium::types::types::{AgentConfig, DomainError};

const BUCKET: &str = "mycelium-agent-config";

#[derive(Serialize, Deserialize)]
struct StoredAgentConfig {
    id: String,
    name: String,
    system_prompt: String,
    model: String,
    tools: Vec<String>,
    max_steps: u32,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
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

fn kv_err(e: wasi::keyvalue::store::Error) -> DomainError {
    DomainError::Backend(format!("kv: {e:?}"))
}

fn open() -> Result<wasi::keyvalue::store::Bucket, DomainError> {
    wasi::keyvalue::store::open(BUCKET).map_err(kv_err)
}

fn k(id: &str) -> String {
    format!("agent/{id}")
}

struct Component;

impl Guest for Component {
    fn create(config: AgentConfig) -> Result<AgentConfig, DomainError> {
        if config.id.trim().is_empty() {
            return Err(DomainError::Validation("agent id required".into()));
        }
        let bucket = open()?;
        if bucket.get(&k(&config.id)).map_err(kv_err)?.is_some() {
            return Err(DomainError::Conflict(format!("agent {} exists", config.id)));
        }
        let stored = StoredAgentConfig::from(&config);
        let bytes =
            serde_json::to_vec(&stored).map_err(|e| DomainError::Internal(e.to_string()))?;
        bucket.set(&k(&config.id), &bytes).map_err(kv_err)?;
        Ok(stored.into())
    }

    fn get(id: String) -> Result<AgentConfig, DomainError> {
        let bucket = open()?;
        match bucket.get(&k(&id)).map_err(kv_err)? {
            Some(bytes) => {
                let stored: StoredAgentConfig = serde_json::from_slice(&bytes)
                    .map_err(|e| DomainError::Internal(e.to_string()))?;
                Ok(stored.into())
            }
            None => Err(DomainError::NotFound(id)),
        }
    }

    fn update(config: AgentConfig) -> Result<AgentConfig, DomainError> {
        let bucket = open()?;
        // Preserve any KV-side extension fields (endpoint, api_key) on the existing record.
        let existing_extras = match bucket.get(&k(&config.id)).map_err(kv_err)? {
            Some(bytes) => serde_json::from_slice::<StoredAgentConfig>(&bytes)
                .ok()
                .map(|s| (s.endpoint, s.api_key))
                .unwrap_or((None, None)),
            None => return Err(DomainError::NotFound(config.id)),
        };
        let mut stored = StoredAgentConfig::from(&config);
        stored.endpoint = existing_extras.0;
        stored.api_key = existing_extras.1;
        let bytes =
            serde_json::to_vec(&stored).map_err(|e| DomainError::Internal(e.to_string()))?;
        bucket.set(&k(&config.id), &bytes).map_err(kv_err)?;
        Ok(stored.into())
    }

    fn delete(id: String) -> Result<(), DomainError> {
        let bucket = open()?;
        bucket.delete(&k(&id)).map_err(kv_err)
    }

    fn list_agents() -> Result<Vec<AgentConfig>, DomainError> {
        let bucket = open()?;
        let resp = bucket.list_keys(None).map_err(kv_err)?;
        let mut out = Vec::new();
        for key in resp.keys.iter().filter(|k| k.starts_with("agent/")) {
            if let Ok(Some(bytes)) = bucket.get(key) {
                if let Ok(stored) = serde_json::from_slice::<StoredAgentConfig>(&bytes) {
                    out.push(stored.into());
                }
            }
        }
        Ok(out)
    }
}

export!(Component);
