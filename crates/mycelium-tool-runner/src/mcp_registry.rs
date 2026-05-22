//! MCP server registry.
//!
//! MCP components export `mycelium:mcp/mcp-provider` — one component can
//! serve N tools. On install the runner calls `list-tools`, writes each
//! tool's schema to `mycelium-tools` (same bucket the agent reads), and
//! stores a tool→server routing map so `handle_call` can dispatch correctly.
//!
//! KV buckets:
//!   * `mycelium-mcp-servers`  — server_name → ComponentManifest JSON
//!   * `mycelium-mcp-tool-map` — tool_name   → server_name
//!
//! `mycelium-tools` entries written here carry an extra `mcp_server` field
//! which is ignored by agent.rs (it only reads `description` and `parameters`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_nats::jetstream::kv::Store;
use futures_util::StreamExt;
use tokio::sync::RwLock;

use crate::{
    sandbox::{McpToolDef, Sandbox},
    Loader, SkillManifest,
};

const SERVERS_BUCKET: &str = "mycelium-mcp-servers";
const TOOL_MAP_BUCKET: &str = "mycelium-mcp-tool-map";
const TOOLS_BUCKET: &str = "mycelium-tools";
const CACHE_TTL: Duration = Duration::from_secs(30);

pub struct McpRegistry {
    servers_kv: Store,
    tool_map_kv: Store,
    tools_kv: Store,
    /// In-memory cache: tool_name → server_name.
    route_cache: RwLock<Option<(Instant, HashMap<String, String>)>>,
}

impl McpRegistry {
    pub async fn open(js: &async_nats::jetstream::Context) -> Result<Self> {
        let servers_kv = open_or_create(js, SERVERS_BUCKET, "MCP server manifests").await?;
        let tool_map_kv = open_or_create(js, TOOL_MAP_BUCKET, "MCP tool-to-server routing map").await?;
        let tools_kv = open_or_create(js, TOOLS_BUCKET, "tool schemas for LLM").await?;
        Ok(Self {
            servers_kv,
            tool_map_kv,
            tools_kv,
            route_cache: RwLock::new(None),
        })
    }

    /// Install an MCP server: fetch wasm, enumerate tools, write schemas.
    pub async fn install(
        &self,
        manifest: &SkillManifest,
        loader: &Loader,
        sandbox: &Sandbox,
        js: &async_nats::jetstream::Context,
    ) -> Result<Vec<McpToolDef>> {
        let bytes = loader.fetch(manifest, js).await?;
        let tools = sandbox
            .list_tools_mcp(manifest, &bytes)
            .await
            .map_err(|e| anyhow::anyhow!("list-tools failed for {}: {e}", manifest.name))?;

        // Persist manifest.
        let manifest_json = serde_json::to_vec(manifest)?;
        self.servers_kv
            .put(&manifest.name, manifest_json.into())
            .await
            .with_context(|| format!("write manifest for {}", manifest.name))?;

        // Write tool schemas + routing entries.
        for tool in &tools {
            let schema: serde_json::Value = serde_json::from_str(&tool.input_schema)
                .unwrap_or_else(|_| {
                    serde_json::json!({"type": "object", "properties": {}, "required": []})
                });
            let entry = serde_json::to_vec(&serde_json::json!({
                "description": tool.description,
                "parameters":  schema,
                "mcp_server":  manifest.name,
            }))?;
            self.tools_kv
                .put(&tool.name, entry.into())
                .await
                .with_context(|| format!("write tool schema for {}", tool.name))?;
            self.tool_map_kv
                .put(&tool.name, manifest.name.as_bytes().to_vec().into())
                .await
                .with_context(|| format!("write tool-map entry for {}", tool.name))?;
        }

        self.invalidate_cache().await;
        tracing::info!(server = %manifest.name, tools = tools.len(), "MCP server installed");
        Ok(tools)
    }

    /// Uninstall an MCP server and remove all its tool entries.
    pub async fn uninstall(&self, server_name: &str) -> Result<()> {
        // Collect tools that belong to this server.
        let mut owned: Vec<String> = Vec::new();
        let mut keys = self.tool_map_kv.keys().await?;
        while let Some(key) = keys.next().await {
            let Ok(key) = key else { continue };
            if let Some(bytes) = self.tool_map_kv.get(&key).await? {
                if bytes.as_ref() == server_name.as_bytes() {
                    owned.push(key);
                }
            }
        }

        for name in &owned {
            let _ = self.tools_kv.delete(name).await;
            let _ = self.tool_map_kv.delete(name).await;
        }
        let _ = self.servers_kv.delete(server_name).await;

        self.invalidate_cache().await;
        tracing::info!(server = %server_name, removed_tools = owned.len(), "MCP server uninstalled");
        Ok(())
    }

    /// Look up which server owns `tool_name`. Returns `None` for non-MCP tools.
    pub async fn route(&self, tool_name: &str) -> Option<String> {
        {
            let cache = self.route_cache.read().await;
            if let Some((when, ref map)) = *cache {
                if when.elapsed() < CACHE_TTL {
                    return map.get(tool_name).cloned();
                }
            }
        }
        self.refresh_cache().await.ok()?.get(tool_name).cloned()
    }

    /// Fetch a server manifest for sandbox invocation.
    pub async fn get_manifest(&self, server_name: &str) -> Result<Option<SkillManifest>> {
        let Some(bytes) = self.servers_kv.get(server_name).await? else {
            return Ok(None);
        };
        Ok(serde_json::from_slice::<SkillManifest>(&bytes).ok())
    }

    async fn invalidate_cache(&self) {
        *self.route_cache.write().await = None;
    }

    async fn refresh_cache(&self) -> Result<HashMap<String, String>> {
        let mut map = HashMap::new();
        let mut keys = self.tool_map_kv.keys().await?;
        while let Some(key) = keys.next().await {
            let Ok(key) = key else { continue };
            if let Some(bytes) = self.tool_map_kv.get(&key).await? {
                if let Ok(server) = std::str::from_utf8(&bytes) {
                    map.insert(key, server.to_string());
                }
            }
        }
        *self.route_cache.write().await = Some((Instant::now(), map.clone()));
        Ok(map)
    }
}

async fn open_or_create(
    js: &async_nats::jetstream::Context,
    bucket: &str,
    description: &str,
) -> Result<Store> {
    match js.get_key_value(bucket).await {
        Ok(s) => Ok(s),
        Err(_) => js
            .create_key_value(async_nats::jetstream::kv::Config {
                bucket: bucket.to_string(),
                description: description.to_string(),
                history: 5,
                ..Default::default()
            })
            .await
            .with_context(|| format!("create KV bucket {bucket}")),
    }
}
