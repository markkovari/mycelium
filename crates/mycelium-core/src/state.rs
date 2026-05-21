use std::sync::Arc;

use anyhow::{Context, Result};
use async_nats::jetstream;

use crate::config::Config;

/// Shared application state. Cloned cheaply into each task — every field is
/// either an `Arc` or already an `async_nats` handle (which is internally
/// cloneable + cheap).
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub nats: async_nats::Client,
    pub js: jetstream::Context,
    pub http: reqwest::Client,
}

impl AppState {
    pub async fn connect(config: Config) -> Result<Self> {
        let nats = async_nats::connect(&config.nats_url)
            .await
            .with_context(|| format!("connect to NATS at {}", config.nats_url))?;
        tracing::info!(nats_url = %config.nats_url, "connected to NATS");

        let js = jetstream::new(nats.clone());

        let http = reqwest::Client::builder()
            .user_agent("mycelium-core/0.1.0")
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .context("build reqwest client")?;

        Ok(Self {
            config: Arc::new(config),
            nats,
            js,
            http,
        })
    }
}
