//! mycelium-core: the trusted-pipeline binary.
//!
//! Replaces the wash-orchestrated wasm components (telegram-poller,
//! channel-router, executor, agent, conversation-store, memory-store,
//! agent-registry, event-logger, router, telegram-out, gateway) with a single
//! tokio process. Cross-module calls that used to be WIT-linked are now plain
//! `async fn` calls; NATS subjects between modules use async-nats JetStream
//! pull consumers with queue groups so wash's per-message-instance fanout
//! cannot multiply work anymore.
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
