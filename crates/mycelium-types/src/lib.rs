//! Shared domain types mirroring the mycelium:types WIT interface.
//! Used by native crates (mycelium-cli, mycelium-tests) that can't import WASM-only WIT bindings.

use serde::{Deserialize, Serialize};

pub type ConversationId = String;
pub type TaskId = String;
pub type RunId = String;
pub type AgentId = String;
pub type ToolId = String;
pub type StepId = String;
pub type MemoryKey = String;
pub type Timestamp = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LifecyclePhase {
    Started,
    Stepping,
    ToolCalls,
    ToolResult,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub run_id: RunId,
    pub task_id: TaskId,
    pub phase: LifecyclePhase,
    pub step: u32,
    pub ts: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub tool_use_id: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub agent_id: AgentId,
    pub title: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub conversation_id: ConversationId,
    pub agent_id: AgentId,
    pub input: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Paused,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub call_id: String,
    pub tool_id: ToolId,
    pub args_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub call_id: String,
    pub tool_id: ToolId,
    pub output_json: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: AgentId,
    pub name: String,
    pub system_prompt: String,
    pub model: String,
    pub tools: Vec<ToolId>,
    pub max_steps: u32,
}

/// Channel message normalised from any inbound channel (Telegram, CLI, …)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub channel_msg_id: String,
    pub sender_id: String,
    pub conversation_id: Option<ConversationId>,
    pub agent_id: Option<AgentId>,
    pub text: String,
    pub raw_json: String,
}

/// Reply to be sent back through a specific channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelReply {
    pub recipient_id: String,
    pub conversation_id: ConversationId,
    pub text: String,
    pub format_hints: Option<String>,
}

/// Pairing code displayed by the CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCode {
    pub code: String,
    pub expires_at: u64,
}

/// Active pairing between a CLI session and a Telegram chat
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairInfo {
    pub session_id: String,
    pub chat_id: String,
    pub conversation_id: ConversationId,
    pub paired_at: u64,
}
