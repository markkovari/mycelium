use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub nats_url: String,
    /// Where to cache fetched skill components on disk.
    pub skill_cache_dir: String,
    /// Approved capabilities. Skills whose manifest declares anything outside
    /// this list are refused at load time.
    #[serde(default)]
    pub approved_capabilities: Vec<String>,
    /// Per-call default fuel ceiling, used when a skill manifest omits its own.
    pub default_fuel_limit: u64,
    /// Per-call default memory cap (bytes).
    pub default_memory_max: u64,
    /// Per-call default wall-clock deadline (milliseconds).
    pub default_wall_deadline_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nats_url: "nats://127.0.0.1:4222".into(),
            skill_cache_dir: "/var/lib/mycelium/skill-cache".into(),
            approved_capabilities: vec![
                "wasi:clocks/wall-clock".into(),
                "wasi:logging/logging".into(),
                "wasi:keyvalue/store@0.2.0-draft".into(),
            ],
            default_fuel_limit: 500_000_000,
            default_memory_max: 16 * 1024 * 1024,
            default_wall_deadline_ms: 30_000,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let _ = dotenvy::dotenv();
        let _ = dotenvy::from_path("/etc/mycelium/secrets.env");

        let mut cfg = if let Ok(path) = std::env::var("MYCELIUM_TOOL_RUNNER_CONFIG") {
            Self::from_toml_file(Path::new(&path))?
        } else {
            Self::default()
        };

        if let Ok(v) = std::env::var("NATS_URL") {
            cfg.nats_url = v;
        }
        if let Ok(v) = std::env::var("MYCELIUM_SKILL_CACHE_DIR") {
            cfg.skill_cache_dir = v;
        }
        if let Ok(v) = std::env::var("MYCELIUM_APPROVED_CAPS") {
            cfg.approved_capabilities = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Ok(v) = std::env::var("MYCELIUM_DEFAULT_FUEL") {
            cfg.default_fuel_limit = v.parse().unwrap_or(cfg.default_fuel_limit);
        }
        if let Ok(v) = std::env::var("MYCELIUM_DEFAULT_MEMORY_MAX") {
            cfg.default_memory_max = v.parse().unwrap_or(cfg.default_memory_max);
        }
        if let Ok(v) = std::env::var("MYCELIUM_DEFAULT_WALL_DEADLINE_MS") {
            cfg.default_wall_deadline_ms = v.parse().unwrap_or(cfg.default_wall_deadline_ms);
        }
        Ok(cfg)
    }

    fn from_toml_file(path: &Path) -> Result<Self> {
        let bytes = std::fs::read_to_string(path)
            .with_context(|| format!("read config file at {}", path.display()))?;
        let parsed: Self =
            toml::from_str(&bytes).with_context(|| format!("parse TOML at {}", path.display()))?;
        Ok(parsed)
    }

    pub fn capability_approved(&self, cap: &str) -> bool {
        self.approved_capabilities.iter().any(|c| c == cap)
    }
}
