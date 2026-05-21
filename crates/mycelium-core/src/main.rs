use anyhow::Result;
use mycelium_core::{
    agent, agent_registry::AgentRegistry, channel_router, config::Config,
    conversation_store::ConversationStore, events, executor, gateway, state::AppState,
    telegram_out, telegram_poller,
};
use tokio::sync::broadcast;
use tokio::task::JoinSet;
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
        disabled = ?config.disabled_modules,
        "mycelium-core starting",
    );

    let state = AppState::connect(config.clone()).await?;
    let registry = AgentRegistry::open(&state.js).await?;
    let convs = ConversationStore::open(&state.js).await?;

    let (shutdown_tx, _) = broadcast::channel::<()>(16);
    let mut tasks = JoinSet::new();

    macro_rules! spawn_module {
        ($name:literal, $task:expr) => {{
            if state.config.module_enabled($name) {
                let fut = $task;
                tasks.spawn(async move {
                    let r: Result<()> = fut.await;
                    if let Err(e) = r {
                        tracing::error!(module = $name, error = %e, "module exited with error");
                    } else {
                        tracing::info!(module = $name, "module exited cleanly");
                    }
                });
                tracing::info!(module = $name, "spawned");
            } else {
                tracing::info!(module = $name, "disabled via config");
            }
        }};
    }

    spawn_module!("events", events::run(state.clone(), shutdown_tx.subscribe()));
    spawn_module!(
        "telegram_poller",
        telegram_poller::run(state.clone(), shutdown_tx.subscribe())
    );
    spawn_module!(
        "channel_router",
        channel_router::run(
            state.clone(),
            registry.clone(),
            convs.clone(),
            shutdown_tx.subscribe()
        )
    );
    spawn_module!(
        "executor",
        executor::run(state.clone(), convs.clone(), shutdown_tx.subscribe())
    );
    spawn_module!(
        "agent",
        agent::run(
            state.clone(),
            registry.clone(),
            convs.clone(),
            shutdown_tx.subscribe()
        )
    );
    spawn_module!(
        "telegram_out",
        telegram_out::run(state.clone(), shutdown_tx.subscribe())
    );
    spawn_module!(
        "gateway",
        gateway::run(
            state.clone(),
            registry.clone(),
            convs.clone(),
            shutdown_tx.subscribe()
        )
    );

    tracing::info!("mycelium-core ready");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("ctrl-c received, draining tasks");
        }
        // If any task panics or otherwise stops on its own, treat as fatal.
        // (Graceful exit is signalled via `tracing::info!("... exited cleanly")`
        // above; we don't break the loop unless ctrl_c fires.)
        _ = wait_for_any_panic(&mut tasks) => {
            tracing::warn!("a module task ended unexpectedly; shutting the rest down");
        }
    }

    let _ = shutdown_tx.send(());
    let drain = tokio::time::timeout(std::time::Duration::from_secs(5), drain_tasks(tasks)).await;
    if drain.is_err() {
        tracing::warn!("some modules did not drain within 5s; aborting");
    }

    drop(state);
    drop(registry);
    drop(convs);
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

async fn wait_for_any_panic(tasks: &mut JoinSet<()>) {
    if let Some(res) = tasks.join_next().await {
        if let Err(e) = res {
            tracing::error!(error = %e, "module task panicked");
        }
    }
}

async fn drain_tasks(mut tasks: JoinSet<()>) {
    while let Some(_res) = tasks.join_next().await {}
}
