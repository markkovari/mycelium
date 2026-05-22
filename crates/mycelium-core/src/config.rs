use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Runtime config for mycelium-core.
///
/// Loaded from environment variables (twelve-factor) with optional override
/// from `MYCELIUM_CONFIG=/path/to/config.toml`. Environment always wins over
/// the file so secrets stay out of the file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// `nats://host:port`. Default: `nats://127.0.0.1:4222`.
    pub nats_url: String,

    /// Telegram bot token. Required for the telegram_poller + executor
    /// progress-edit path. May be empty in dev.
    pub telegram_bot_token: String,

    /// Default agent id when a chat has no per-chat override and no admin
    /// has run `/agent set`. `"auto"` falls back to "first agent in registry".
    pub default_agent_id: String,

    /// Gemini-compat LLM endpoint (OpenAI surface).
    pub llm_endpoint: String,

    /// Default model name when AgentConfig.model is unset.
    pub llm_model: String,

    /// Gemini API key.
    pub llm_api_key: String,

    /// Requests-per-minute soft cap. 0 disables.
    pub llm_rpm: u64,

    /// Requests-per-day hard cap. 0 disables.
    pub llm_rpd: u64,

    /// HTTP gateway listen address. `axum` binds here when the gateway
    /// module is enabled. Empty disables the gateway.
    pub gateway_listen: String,

    /// Comma-separated module names to disable at startup.
    pub disabled_modules: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nats_url: "nats://127.0.0.1:4222".into(),
            telegram_bot_token: String::new(),
            default_agent_id: "auto".into(),
            llm_endpoint:
                "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
                    .into(),
            llm_model: "gemini-2.5-flash-lite".into(),
            llm_api_key: String::new(),
            llm_rpm: 10,
            llm_rpd: 200,
            gateway_listen: String::new(),
            disabled_modules: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        // Best-effort .env load — silently no-op if absent.
        let _ = dotenvy::dotenv();
        let _ = dotenvy::from_path("/etc/mycelium/secrets.env");

        let mut cfg = if let Ok(path) = std::env::var("MYCELIUM_CONFIG") {
            Self::from_toml_file(Path::new(&path))?
        } else {
            Self::default()
        };

        // Env overrides (always wins).
        if let Ok(v) = std::env::var("NATS_URL") {
            cfg.nats_url = v;
        }
        if let Ok(v) = std::env::var("TELEGRAM_BOT_TOKEN") {
            cfg.telegram_bot_token = v;
        }
        if let Ok(v) = std::env::var("DEFAULT_AGENT_ID") {
            cfg.default_agent_id = v;
        }
        if let Ok(v) = std::env::var("LLM_ENDPOINT") {
            cfg.llm_endpoint = v;
        }
        if let Ok(v) = std::env::var("LLM_MODEL") {
            cfg.llm_model = v;
        }
        if let Ok(v) = std::env::var("LLM_API_KEY") {
            cfg.llm_api_key = v;
        }
        if let Ok(v) = std::env::var("LLM_RPM") {
            cfg.llm_rpm = v.parse().unwrap_or(cfg.llm_rpm);
        }
        if let Ok(v) = std::env::var("LLM_RPD") {
            cfg.llm_rpd = v.parse().unwrap_or(cfg.llm_rpd);
        }
        if let Ok(v) = std::env::var("GATEWAY_LISTEN") {
            cfg.gateway_listen = v;
        }
        if let Ok(v) = std::env::var("MYCELIUM_DISABLED_MODULES") {
            cfg.disabled_modules = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
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

    pub fn module_enabled(&self, name: &str) -> bool {
        !self.disabled_modules.iter().any(|m| m == name)
    }
}
