//! mycelium-core: the trusted-pipeline binary.
//!
//! Single tokio process containing all pipeline modules (telegram-poller,
//! channel-router, executor, agent, conversation-store, memory-store,
//! agent-registry, event-logger, router, telegram-out, gateway). Cross-module
//! calls are plain `async fn`; NATS subjects use JetStream pull consumers with
//! queue groups for single delivery.
//!
//! Skill execution lives in a separate process — see `mycelium-tool-runner`.

pub mod agent;
pub mod agent_registry;
pub mod channel_router;
pub mod compaction;
pub mod config;
pub mod conversation_store;
pub mod events;
pub mod executor;
pub mod gateway;
pub mod hooks;
pub mod memory_store;
pub mod session;
pub mod state;
pub mod telegram;
pub mod telegram_out;
pub mod telegram_poller;
