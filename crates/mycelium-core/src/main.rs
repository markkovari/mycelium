use anyhow::Result;
use mycelium_core::{
    agent, agent_registry::AgentRegistry, channel_router, config::Config,
    conversation_store::ConversationStore, events, executor, gateway, hooks,
    session::SessionStore, state::AppState, telegram_out, telegram_poller,
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
    let sessions = SessionStore::open(&state.js).await?;

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
            sessions.clone(),
            shutdown_tx.subscribe()
        )
    );
    spawn_module!(
        "session_sweeper",
        session_sweeper(sessions.clone(), state.clone(), shutdown_tx.subscribe())
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
        // Only treat panics as fatal. Modules that finish cleanly (e.g. gateway
        // disabled via empty GATEWAY_LISTEN) are expected; the rest of the
        // pipeline keeps running.
        _ = wait_for_panic(&mut tasks) => {
            tracing::warn!("a module task panicked; shutting the rest down");
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
    drop(sessions);
    Ok(())
}

async fn session_sweeper(
    sessions: SessionStore,
    state: AppState,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.recv() => break,
            _ = tick.tick() => {
                match sessions.sweep_idle().await {
                    Ok(closed) if !closed.is_empty() => {
                        tracing::info!(count = closed.len(), "closed idle sessions");
                        for sid in &closed {
                            let _ = hooks::fire(
                                &state.nats,
                                "session-end",
                                serde_json::json!({"session_id": sid, "reason": "idle-timeout"}),
                            )
                            .await;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "session sweep failed"),
                }
            }
        }
    }
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

async fn wait_for_panic(tasks: &mut JoinSet<()>) {
    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(()) => {
                // Module returned Ok — likely disabled by config. Keep waiting.
                continue;
            }
            Err(e) if e.is_panic() => {
                tracing::error!(error = %e, "module task panicked");
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, "module task ended with non-panic error");
                continue;
            }
        }
    }
}

async fn drain_tasks(mut tasks: JoinSet<()>) {
    while let Some(_res) = tasks.join_next().await {}
}
