//! Generic wasm-component manifest. Lifted from the original
//! `mycelium-tool-runner::manifest::SkillManifest`; same schema, broader name.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentManifest {
    pub name: String,
    pub version: String,
    /// Origin of the wasm binary. Runner tries: cache → fetch.
    pub source: ComponentSource,
    /// WIT-import strings this component needs (e.g. `wasi:http/outgoing-handler`).
    /// Runner refuses to load if any are not in the operator allow-list.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Per-call ceiling. Falls back to the runner default when absent.
    #[serde(default)]
    pub fuel_limit: Option<u64>,
    #[serde(default)]
    pub memory_max_bytes: Option<u64>,
    #[serde(default)]
    pub wall_deadline_ms: Option<u64>,
    /// Free-form description for human/LLM tool listings.
    #[serde(default)]
    pub description: String,
    /// JSON schema for the component's input. Mirrors OpenAI function-calling
    /// `parameters` shape. Kept generic so hook manifests can use it too.
    #[serde(default = "default_parameters")]
    pub parameters: serde_json::Value,
}

fn default_parameters() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}, "required": []})
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ComponentSource {
    Oci {
        image: String,
        #[serde(default)]
        sha256: Option<String>,
    },
    NatsObjectStore {
        bucket: String,
        key: String,
        sha256: String,
    },
    File {
        path: String,
        #[serde(default)]
        sha256: Option<String>,
    },
}
