//! LLM step executor.
//!
//! Native rewrite of `components/agent/src/lib.rs`. Subscribes to
//! `mycelium.task.step.agent`, loads the agent config + conversation history,
//! enforces RPM/RPD caps and the circuit breaker, calls the LLM via reqwest,
//! and publishes either `mycelium.step.tool-calls` (function calls) or
//! `mycelium.step.result` (final text / error).
//!
//! The wasi:http boilerplate + wasi:io/poll-based timeouts collapse into
//! plain `reqwest::Client::post(...)?.timeout(...)`. The marker-CAS
//! `claim_step` dedup is dropped — native single-instance per subject.

use std::time::Duration;

use anyhow::{Context, Result};
use async_nats::jetstream::kv::Store;
use futures_util::StreamExt;
use mycelium_types::MessageRole;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::{
    agent_registry::AgentRegistry, compaction, conversation_store::ConversationStore, hooks,
    state::AppState,
};

const STEP_AGENT: &str = "mycelium.task.step.agent";
const STEP_RESULT: &str = "mycelium.step.result";
const STEP_TOOL_CALLS: &str = "mycelium.step.tool-calls";
const STATE_BUCKET: &str = "mycelium-task-state";
const TOOLS_BUCKET: &str = "mycelium-tools";

const CIRCUIT_OPEN_UNTIL_KEY: &str = "circuit/open-until";
const CIRCUIT_FAILS_KEY: &str = "circuit/fails";
const CIRCUIT_FAIL_THRESHOLD: u64 = 5;
const CIRCUIT_COOLDOWN_S: u64 = 60;

pub async fn run(
    state: AppState,
    registry: AgentRegistry,
    convs: ConversationStore,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    let state_kv = state
        .js
        .get_key_value(STATE_BUCKET)
        .await
        .with_context(|| format!("open {STATE_BUCKET}"))?;
    let tools_kv = state
        .js
        .get_key_value(TOOLS_BUCKET)
        .await
        .with_context(|| format!("open {TOOLS_BUCKET}"))?;

    let mut sub = state
        .nats
        .subscribe(STEP_AGENT.to_string())
        .await
        .with_context(|| format!("subscribe {STEP_AGENT}"))?;
    tracing::info!(subject = STEP_AGENT, "agent task started");

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                tracing::info!("agent task shutting down");
                break;
            }
            Some(msg) = sub.next() => {
                let state2 = state.clone();
                let registry2 = registry.clone();
                let convs2 = convs.clone();
                let state_kv2 = state_kv.clone();
                let tools_kv2 = tools_kv.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_step(&state2, &registry2, &convs2, &state_kv2, &tools_kv2, &msg.payload).await {
                        tracing::warn!(error = %e, "agent step failed");
                    }
                });
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct StepReq {
    task_id: String,
    conversation_id: String,
    agent_id: String,
    #[serde(default)]
    run_id: String,
}

#[derive(Deserialize)]
struct StreamCtxState {
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    progress_message_id: i64,
}

async fn handle_step(
    state: &AppState,
    registry: &AgentRegistry,
    convs: &ConversationStore,
    state_kv: &Store,
    tools_kv: &Store,
    body: &[u8],
) -> Result<()> {
    let req: StepReq = serde_json::from_slice(body).context("decode step.agent")?;

    let stored = registry.get_stored(&req.agent_id).await.unwrap_or_default();
    let system_prompt = stored
        .as_ref()
        .map(|s| s.system_prompt.clone())
        .unwrap_or_else(|| format!("You are mycelium agent {}.", req.agent_id));
    let model = stored
        .as_ref()
        .map(|s| s.model.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| state.config.llm_model.clone());
    let endpoint = stored
        .as_ref()
        .and_then(|s| s.endpoint.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| state.config.llm_endpoint.clone());
    let api_key = stored
        .as_ref()
        .and_then(|s| s.api_key.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| state.config.llm_api_key.clone());
    let stored_for_compact = stored.clone();
    let tool_names: Vec<String> = stored.map(|s| s.tools).unwrap_or_default();

    // before-prompt-build hook: operator can inject extra context or block
    // the step entirely.
    let pb_outcome = hooks::fire(
        &state.nats,
        "before-prompt-build",
        json!({
            "task_id": req.task_id,
            "run_id": req.run_id,
            "conversation_id": req.conversation_id,
            "agent_id": req.agent_id,
            "system_prompt": system_prompt,
        }),
    )
    .await;
    if pb_outcome.block {
        return publish_result(
            state,
            &req.task_id,
            None,
            Some(format!(
                "before-prompt-build blocked: {}",
                pb_outcome.reason.unwrap_or_default()
            )),
        )
        .await;
    }
    let system_prompt = pb_outcome
        .payload
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .unwrap_or(&system_prompt)
        .to_string();

    // Build the messages array: system + (possibly compacted) full history.
    let mut messages: Vec<Value> = vec![json!({"role": "system", "content": system_prompt})];
    match convs.get_messages(&req.conversation_id).await {
        Ok(history) => {
            let compaction_threshold = stored_for_compact
                .as_ref()
                .and_then(|s| s.compaction_threshold)
                .map(|n| n as usize)
                .filter(|&n| n > 0)
                .unwrap_or(compaction::DEFAULT_MSG_THRESHOLD);
            let summary_model = stored_for_compact
                .as_ref()
                .and_then(|s| s.compaction_model.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| model.clone());
            let history = compaction::maybe_compact(
                state,
                convs,
                &req.conversation_id,
                &req.agent_id,
                history,
                compaction_threshold,
                &summary_model,
                &api_key,
                &endpoint,
            )
            .await;
            for m in history {
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                };
                messages.push(json!({"role": role, "content": m.content}));
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "get_messages failed");
        }
    }

    // Circuit breaker.
    if let Some(cooldown) = circuit_open(state_kv).await {
        return publish_result(
            state,
            &req.task_id,
            None,
            Some(format!(
                "circuit open; retry in {cooldown}s after repeated LLM failures"
            )),
        )
        .await;
    }

    // Per-day cap.
    if let Err(over) = rpd_check(state, state_kv, &req.agent_id).await {
        return publish_result(
            state,
            &req.task_id,
            None,
            Some(format!(
                "daily LLM budget exceeded ({over} over cap); resets at UTC midnight"
            )),
        )
        .await;
    }
    // Per-minute cap.
    if let Err(over) = rpm_check(state, state_kv, &req.agent_id).await {
        return publish_result(
            state,
            &req.task_id,
            None,
            Some(format!(
                "rate limit hit ({over} over budget this minute); try again shortly"
            )),
        )
        .await;
    }

    let tool_specs = load_tool_specs(tools_kv, &tool_names).await;

    // before-agent-reply hook: last chance to mutate model/tools or claim
    // the turn synthetically. A block here aborts the LLM call entirely.
    let bar_outcome = hooks::fire(
        &state.nats,
        "before-agent-reply",
        json!({
            "task_id": req.task_id,
            "run_id": req.run_id,
            "agent_id": req.agent_id,
            "model": model,
            "tool_count": tool_specs.len(),
        }),
    )
    .await;
    if bar_outcome.block {
        return publish_result(
            state,
            &req.task_id,
            None,
            Some(format!(
                "before-agent-reply blocked: {}",
                bar_outcome.reason.unwrap_or_default()
            )),
        )
        .await;
    }

    let stream_ctx: StreamCtxState = state_kv
        .get(format!("task/{}", req.task_id))
        .await
        .ok()
        .flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(StreamCtxState {
            chat_id: String::new(),
            progress_message_id: 0,
        });

    match call_llm_streaming(
        &state.http,
        &state.nats,
        &endpoint,
        &model,
        &api_key,
        &messages,
        &tool_specs,
        &req.run_id,
        &req.task_id,
        &stream_ctx.chat_id,
        stream_ctx.progress_message_id,
    )
    .await
    {
        Ok(LlmReply::Tools(calls)) => {
            note_circuit_success(state_kv).await;
            let _ = hooks::fire(
                &state.nats,
                "agent-end",
                json!({
                    "task_id": req.task_id,
                    "run_id": req.run_id,
                    "kind": "tools",
                    "call_count": calls.len(),
                }),
            )
            .await;
            let payload = json!({
                "task_id": req.task_id,
                "conversation_id": req.conversation_id,
                "agent_id": req.agent_id,
                "calls": calls,
            });
            state
                .nats
                .publish(STEP_TOOL_CALLS.to_string(), serde_json::to_vec(&payload)?.into())
                .await?;
            Ok(())
        }
        Ok(LlmReply::Text(content)) => {
            note_circuit_success(state_kv).await;
            let _ = hooks::fire(
                &state.nats,
                "agent-end",
                json!({
                    "task_id": req.task_id,
                    "run_id": req.run_id,
                    "kind": "text",
                    "len": content.chars().count(),
                }),
            )
            .await;
            publish_result(state, &req.task_id, Some(content), None).await
        }
        Err(e) => {
            note_circuit_failure(state_kv).await;
            publish_result(state, &req.task_id, None, Some(e.to_string())).await
        }
    }
}

async fn publish_result(
    state: &AppState,
    task_id: &str,
    output: Option<String>,
    error: Option<String>,
) -> Result<()> {
    let body = json!({
        "task_id": task_id,
        "output": output,
        "error": error,
    });
    state
        .nats
        .publish(STEP_RESULT.to_string(), serde_json::to_vec(&body)?.into())
        .await?;
    Ok(())
}

// ─────────────────────────── rate limit ───────────────────────────

async fn bump_counter(kv: &Store, key: &str) -> u64 {
    let current = kv
        .get(key)
        .await
        .ok()
        .flatten()
        .and_then(|b| std::str::from_utf8(&b).ok().map(str::to_string))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let next = current + 1;
    let _ = kv
        .put(key, next.to_string().into_bytes().into())
        .await;
    next
}

async fn rpm_check(state: &AppState, kv: &Store, agent_id: &str) -> Result<u64, u64> {
    let rpm = state.config.llm_rpm;
    if rpm == 0 {
        return Ok(u64::MAX);
    }
    let minute = (chrono::Utc::now().timestamp() / 60) as u64;
    let key = format!("rate/{agent_id}/min/{minute}");
    let count = bump_counter(kv, &key).await;
    if count <= rpm {
        Ok(rpm - count)
    } else {
        Err(count - rpm)
    }
}

async fn rpd_check(state: &AppState, kv: &Store, agent_id: &str) -> Result<u64, u64> {
    let rpd = state.config.llm_rpd;
    if rpd == 0 {
        return Ok(u64::MAX);
    }
    let day = (chrono::Utc::now().timestamp() / 86_400) as u64;
    let key = format!("rate/{agent_id}/day/{day}");
    let count = bump_counter(kv, &key).await;
    if count <= rpd {
        Ok(rpd - count)
    } else {
        Err(count - rpd)
    }
}

async fn circuit_open(kv: &Store) -> Option<u64> {
    let bytes = kv.get(CIRCUIT_OPEN_UNTIL_KEY).await.ok().flatten()?;
    let s = std::str::from_utf8(&bytes).ok()?;
    let until: u64 = s.parse().ok()?;
    let now = chrono::Utc::now().timestamp() as u64;
    if until > now {
        Some(until - now)
    } else {
        None
    }
}

async fn note_circuit_failure(kv: &Store) {
    let n = bump_counter(kv, CIRCUIT_FAILS_KEY).await;
    if n >= CIRCUIT_FAIL_THRESHOLD {
        let until = (chrono::Utc::now().timestamp() as u64) + CIRCUIT_COOLDOWN_S;
        let _ = kv
            .put(
                CIRCUIT_OPEN_UNTIL_KEY,
                until.to_string().into_bytes().into(),
            )
            .await;
        let _ = kv
            .put(CIRCUIT_FAILS_KEY, b"0".to_vec().into())
            .await;
    }
}

async fn note_circuit_success(kv: &Store) {
    let _ = kv.put(CIRCUIT_FAILS_KEY, b"0".to_vec().into()).await;
}

// ─────────────────────────── tool specs ───────────────────────────

async fn load_tool_specs(kv: &Store, wanted: &[String]) -> Vec<Value> {
    // Empty list means "all registered tools".
    if wanted.is_empty() {
        return load_all_tool_specs(kv).await;
    }
    let mut out = Vec::with_capacity(wanted.len());
    for name in wanted {
        let Ok(Some(bytes)) = kv.get(name).await else { continue };
        let Ok(spec) = serde_json::from_slice::<Value>(&bytes) else { continue };
        let description = spec.get("description").cloned().unwrap_or_else(|| json!(""));
        let parameters = spec
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object"}));
        out.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
            }
        }));
    }
    out
}

async fn load_all_tool_specs(kv: &Store) -> Vec<Value> {
    let Ok(mut keys) = kv.keys().await else { return Vec::new() };
    let mut out = Vec::new();
    while let Some(key) = keys.next().await {
        let Ok(name) = key else { continue };
        let Ok(Some(bytes)) = kv.get(&name).await else { continue };
        let Ok(spec) = serde_json::from_slice::<Value>(&bytes) else { continue };
        let description = spec.get("description").cloned().unwrap_or_else(|| json!(""));
        let parameters = spec
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object"}));
        out.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
            }
        }));
    }
    out
}

// ─────────────────────────── LLM call ───────────────────────────

#[derive(Debug)]
enum LlmReply {
    Text(String),
    Tools(Vec<ToolCall>),
}

#[derive(Debug, serde::Serialize)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[allow(clippy::too_many_arguments)]
async fn call_llm_streaming(
    http: &reqwest::Client,
    nats: &async_nats::Client,
    endpoint: &str,
    model: &str,
    api_key: &str,
    messages: &[Value],
    tools: &[Value],
    run_id: &str,
    task_id: &str,
    chat_id: &str,
    progress_message_id: i64,
) -> Result<LlmReply> {
    const MAX_ATTEMPTS: u32 = 4;
    const MAX_DELAY_S: u64 = 60;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": true,
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
            body["tool_choice"] = json!("auto");
        }
        let mut req = http
            .post(endpoint)
            .json(&body)
            .timeout(Duration::from_secs(75));
        if !api_key.is_empty() {
            req = req.bearer_auth(api_key);
        }
        let resp = req.send().await.context("LLM send")?;
        let status = resp.status();
        if !status.is_success() {
            let bytes = resp.bytes().await.context("LLM body")?;
            let retryable = status.as_u16() == 429 || status.is_server_error();
            if retryable && attempt < MAX_ATTEMPTS {
                let delay = parse_retry_delay_seconds(&bytes)
                    .unwrap_or_else(|| 1u64 << (attempt - 1))
                    .min(MAX_DELAY_S);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
            let snippet = String::from_utf8_lossy(&bytes);
            let snippet_chars: String = snippet.chars().take(400).collect();
            return Err(anyhow::anyhow!(
                "HTTP {}: {}",
                status.as_u16(),
                snippet_chars
            ));
        }

        let reply = consume_sse_stream(
            resp,
            nats,
            run_id,
            task_id,
            chat_id,
            progress_message_id,
        )
        .await?;
        return Ok(reply);
    }
}

/// Read an OpenAI-compatible SSE stream from `resp`. Each `data: {...}` line
/// is parsed; text deltas are republished to NATS for streaming consumers
/// (telegram_out debouncer, gateway SSE bridge). Tool-call deltas accumulate
/// by `index` and the final assembled set is returned as `LlmReply::Tools`.
async fn consume_sse_stream(
    resp: reqwest::Response,
    nats: &async_nats::Client,
    run_id: &str,
    task_id: &str,
    chat_id: &str,
    progress_message_id: i64,
) -> Result<LlmReply> {
    use std::collections::BTreeMap;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut text_acc = String::new();
    let mut tool_acc: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
    let mut seq: u64 = 0;
    let subject = if run_id.is_empty() {
        String::new()
    } else {
        format!("mycelium.run.{run_id}.assistant")
    };
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("LLM stream chunk")?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = buf.find("\n\n") {
            let event = buf[..idx].to_string();
            buf.drain(..idx + 2);
            for line in event.lines() {
                let Some(payload) = line.strip_prefix("data:").map(str::trim) else {
                    continue;
                };
                if payload == "[DONE]" {
                    continue;
                }
                let Ok(parsed) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };
                let Some(delta) = parsed.pointer("/choices/0/delta") else {
                    continue;
                };
                if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        text_acc.push_str(text);
                        seq += 1;
                        if !subject.is_empty() {
                            let env = json!({
                                "delta": text,
                                "done": false,
                                "seq": seq,
                                "task_id": task_id,
                                "chat_id": chat_id,
                                "progress_message_id": progress_message_id,
                            });
                            if let Ok(bytes) = serde_json::to_vec(&env) {
                                let _ = nats.publish(subject.clone(), bytes.into()).await;
                            }
                        }
                    }
                }
                if let Some(arr) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in arr {
                        let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0)
                            as usize;
                        let entry = tool_acc.entry(index).or_default();
                        if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                            if !id.is_empty() {
                                entry.0 = id.to_string();
                            }
                        }
                        if let Some(name) =
                            tc.pointer("/function/name").and_then(|v| v.as_str())
                        {
                            if !name.is_empty() {
                                entry.1 = name.to_string();
                            }
                        }
                        if let Some(args) =
                            tc.pointer("/function/arguments").and_then(|v| v.as_str())
                        {
                            entry.2.push_str(args);
                        }
                    }
                }
            }
        }
    }
    if !subject.is_empty() {
        let env = json!({
            "delta": "",
            "done": true,
            "seq": seq + 1,
            "task_id": task_id,
            "chat_id": chat_id,
            "progress_message_id": progress_message_id,
        });
        if let Ok(bytes) = serde_json::to_vec(&env) {
            let _ = nats.publish(subject, bytes.into()).await;
        }
    }
    if !tool_acc.is_empty() {
        let calls: Vec<ToolCall> = tool_acc
            .into_values()
            .filter(|(_, name, _)| !name.is_empty())
            .map(|(id, name, arguments)| ToolCall {
                id,
                name,
                arguments: if arguments.is_empty() {
                    "{}".into()
                } else {
                    arguments
                },
            })
            .collect();
        if !calls.is_empty() {
            return Ok(LlmReply::Tools(calls));
        }
    }
    Ok(LlmReply::Text(text_acc))
}

fn parse_retry_delay_seconds(body: &[u8]) -> Option<u64> {
    let parsed: Value = serde_json::from_slice(body).ok()?;
    let err = match &parsed {
        Value::Array(a) => a.first()?.get("error")?,
        Value::Object(_) => parsed.get("error")?,
        _ => return None,
    };
    let details = err.get("details")?.as_array()?;
    for d in details {
        if let Some(rd) = d.get("retryDelay").and_then(|v| v.as_str()) {
            let n: f64 = rd.trim_end_matches('s').parse().ok()?;
            return Some((n.ceil() as u64).max(1));
        }
    }
    None
}
