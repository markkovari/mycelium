//! Runtime config for mycelium-hook-runner. Same shape as tool-runner;
//! different default cache dir + env prefix.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub nats_url: String,
    pub hook_cache_dir: String,
    #[serde(default)]
    pub approved_capabilities: Vec<String>,
    pub default_fuel_limit: u64,
    pub default_memory_max: u64,
    pub default_wall_deadline_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nats_url: "nats://127.0.0.1:4222".into(),
            hook_cache_dir: "/var/lib/mycelium/hook-cache".into(),
            approved_capabilities: vec![
                "wasi:clocks/wall-clock".into(),
                "wasi:logging/logging".into(),
            ],
            default_fuel_limit: 200_000_000,
            default_memory_max: 8 * 1024 * 1024,
            // Hooks run on the request path — keep the deadline tight.
            default_wall_deadline_ms: 1_500,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let _ = dotenvy::dotenv();
        let _ = dotenvy::from_path("/etc/mycelium/secrets.env");

        let mut cfg = if let Ok(path) = std::env::var("MYCELIUM_HOOK_RUNNER_CONFIG") {
            Self::from_toml_file(Path::new(&path))?
        } else {
            Self::default()
        };

        if let Ok(v) = std::env::var("NATS_URL") {
            cfg.nats_url = v;
        }
        if let Ok(v) = std::env::var("MYCELIUM_HOOK_CACHE_DIR") {
            cfg.hook_cache_dir = v;
        }
        if let Ok(v) = std::env::var("MYCELIUM_HOOK_APPROVED_CAPS") {
            cfg.approved_capabilities = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Ok(v) = std::env::var("MYCELIUM_HOOK_DEFAULT_FUEL") {
            cfg.default_fuel_limit = v.parse().unwrap_or(cfg.default_fuel_limit);
        }
        if let Ok(v) = std::env::var("MYCELIUM_HOOK_DEFAULT_MEMORY_MAX") {
            cfg.default_memory_max = v.parse().unwrap_or(cfg.default_memory_max);
        }
        if let Ok(v) = std::env::var("MYCELIUM_HOOK_DEFAULT_WALL_DEADLINE_MS") {
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
