//! Skill manifest format. Loaded per skill from the `mycelium-skills` KV
//! bucket or co-located with the wasm component.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub version: String,
    /// Origin of the wasm binary. The runner tries this in order: cache → fetch.
    pub source: SkillSource,
    /// WIT-import strings this skill needs (e.g. `wasi:http/outgoing-handler`).
    /// Runner refuses to load if any are not in `Config::approved_capabilities`.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Optional override of the runner default.
    #[serde(default)]
    pub fuel_limit: Option<u64>,
    #[serde(default)]
    pub memory_max_bytes: Option<u64>,
    #[serde(default)]
    pub wall_deadline_ms: Option<u64>,
    /// Free-form text shown to users / used by the LLM in tool listings.
    #[serde(default)]
    pub description: String,
    /// JSON schema for the `arguments` field of a tool-call request.
    /// Mirrors the OpenAI function-calling `parameters` shape.
    #[serde(default = "default_parameters")]
    pub parameters: serde_json::Value,
}

fn default_parameters() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}, "required": []})
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SkillSource {
    /// OCI image reference, e.g. `ghcr.io/markkovari/mycelium/skill-web-fetch:0.1.0`.
    /// The runner uses `oci-distribution` to pull. Anonymous + bearer-token
    /// flows supported; registries that demand creds need them in the env.
    Oci {
        image: String,
        /// Optional SHA-256 of the wasm artifact. If absent, the registry's
        /// content-addressing (digest pinning via the tag) is the only check.
        #[serde(default)]
        sha256: Option<String>,
    },
    /// NATS Object Store. Useful for skills that shouldn't leave the lattice
    /// (no public registry account) or air-gapped deploys. SHA-256 mandatory.
    NatsObjectStore {
        bucket: String,
        key: String,
        sha256: String,
    },
    /// Bare wasm file on disk. Useful in dev. SHA-256 optional but recommended.
    File {
        path: String,
        #[serde(default)]
        sha256: Option<String>,
    },
}
