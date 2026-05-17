//! LLM call logic shared between the agent WASM component and tests.
//! All HTTP calls go through the caller's client (either wasi:http or reqwest).

use anyhow::Result;
use mycelium_types::{AgentConfig, Message, MessageRole, ToolCallRequest, ToolCallResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Build an OpenAI-compatible messages array from the conversation history.
pub fn build_messages(config: &AgentConfig, history: &[Message]) -> Vec<Value> {
    let mut messages = vec![json!({
        "role": "system",
        "content": config.system_prompt
    })];

    for msg in history {
        let role = match msg.role {
            MessageRole::System    => "system",
            MessageRole::User      => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool      => "tool",
        };
        let mut obj = json!({ "role": role, "content": msg.content });
        if let Some(ref id) = msg.tool_use_id {
            obj["tool_call_id"] = json!(id);
        }
        messages.push(obj);
    }

    messages
}

/// Minimal OpenAI-compatible chat completion response
#[derive(Debug, Deserialize)]
pub struct ChatCompletion {
    pub choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message:      AssistantMessage,
    pub finish_reason: String,
}

#[derive(Debug, Deserialize)]
pub struct AssistantMessage {
    pub content:    Option<String>,
    pub tool_calls: Option<Vec<RawToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct RawToolCall {
    pub id:       String,
    pub function: RawFunction,
}

#[derive(Debug, Deserialize)]
pub struct RawFunction {
    pub name:      String,
    pub arguments: String,
}

/// Parse raw tool calls from a chat completion into ToolCallRequests.
pub fn extract_tool_calls(calls: &[RawToolCall]) -> Vec<ToolCallRequest> {
    calls
        .iter()
        .map(|c| ToolCallRequest {
            call_id:   c.id.clone(),
            tool_id:   c.function.name.clone(),
            args_json: c.function.arguments.clone(),
        })
        .collect()
}

/// Serialize tool results into the format expected by the LLM.
pub fn tool_results_to_messages(results: &[ToolCallResult]) -> Vec<Value> {
    results
        .iter()
        .map(|r| {
            json!({
                "role": "tool",
                "tool_call_id": r.call_id,
                "content": r.output_json
            })
        })
        .collect()
}
