use anyhow::Result;
use mycelium_core::{config::Config, state::AppState};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::load()?;
    tracing::info!(
        nats_url = %config.nats_url,
        default_agent = %config.default_agent_id,
        llm_rpm = config.llm_rpm,
        llm_rpd = config.llm_rpd,
        "mycelium-core starting",
    );

    let state = AppState::connect(config).await?;

    // Shutdown broadcast: every module subscribes a receiver and exits its
    // `run` future when this fires. Capacity 16 is generous; only main sends.
    let (shutdown_tx, _) = broadcast::channel::<()>(16);

    // Modules will be wired in subsequent phases. For Phase 1 we just block
    // on the shutdown signal so the binary stays alive and exits cleanly.
    tracing::info!("no modules enabled yet (Phase 1 skeleton); waiting for SIGINT");

    let mut shutdown_rx = shutdown_tx.subscribe();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("ctrl-c received, shutting down");
        }
        _ = shutdown_rx.recv() => {
            tracing::info!("internal shutdown received");
        }
    }

    let _ = shutdown_tx.send(());
    // Give modules a moment to drain — placeholder for future task joins.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Suppress unused-warning until modules land.
    drop(state);

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,mycelium_core=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
