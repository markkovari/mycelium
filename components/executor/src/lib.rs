// Orchestration engine.
// Subscribes to mycelium.task.submit; drives the agent step loop by publishing to mycelium.task.step.agent.
// Persists TaskState to mycelium-task-state KV bucket after every transition.
//
// Message shapes (JSON):
//   mycelium.task.submit body:  Task { id, conversation_id, agent_id, input, created_at }
//   mycelium.task.step.agent body: { task_id, conversation_id, agent_id }
//   mycelium.step.result body:  { task_id, output, error? }
wit_bindgen::generate!({
    path: "wit",
    world: "executor",
    generate_all,
});

use serde::{Deserialize, Serialize};

const BUCKET: &str = "mycelium-task-state";

#[derive(Serialize, Deserialize, Clone)]
struct TaskJson {
    id: String,
    conversation_id: String,
    agent_id: String,
    input: String,
    created_at: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ToolCallSpec {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ToolResultRec {
    id: String,
    name: String,
    output: Option<String>,
    error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct TaskStateJson {
    task: TaskJson,
    status: String,
    output: Option<String>,
    error: Option<String>,
    #[serde(default)]
    step: u32,
    #[serde(default)]
    pending_tool_calls: Vec<ToolCallSpec>,
    #[serde(default)]
    tool_results: Vec<ToolResultRec>,
    #[serde(default)]
    progress_message_id: i64,
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    max_steps: u32,
}

#[derive(Serialize, Deserialize)]
struct StepResult {
    task_id: String,
    output: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct StepRequest<'a> {
    task_id: &'a str,
    conversation_id: &'a str,
    agent_id: &'a str,
}

fn open() -> Result<wasi::keyvalue::store::Bucket, String> {
    wasi::keyvalue::store::open(BUCKET).map_err(|e| format!("{e:?}"))
}

fn save_state(state: &TaskStateJson) -> Result<(), String> {
    let bucket = open()?;
    let json = serde_json::to_vec(state).map_err(|e| e.to_string())?;
    bucket
        .set(&format!("task/{}", state.task.id), &json)
        .map_err(|e| format!("{e:?}"))
}

/// Marker-CAS claim on task.submit. wash 2.1.0 fans the message to multiple
/// component instances; only one should send the Telegram placeholder and
/// publish the first step.agent. Write a random claim token at claim/<id>,
/// read it back, and proceed only if our token survived the race.
fn claim_task(task_id: &str) -> bool {
    let Ok(bucket) = wasi::keyvalue::store::open(BUCKET) else { return true };
    let key = format!("claim/task/{task_id}");
    if matches!(bucket.exists(&key), Ok(true)) {
        return false;
    }
    let token_bytes = wasi::random::random::get_random_bytes(16);
    let token = token_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if bucket.set(&key, token.as_bytes()).is_err() {
        return false;
    }
    match bucket.get(&key) {
        Ok(Some(stored)) if stored == token.as_bytes() => true,
        _ => false,
    }
}

fn load_state(id: &str) -> Result<Option<TaskStateJson>, String> {
    let bucket = open()?;
    match bucket
        .get(&format!("task/{id}"))
        .map_err(|e| format!("{e:?}"))?
    {
        Some(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
        )),
        None => Ok(None),
    }
}

fn cfg(key: &str) -> Option<String> {
    wasi::config::store::get(key).ok().flatten()
}

#[derive(Deserialize)]
struct PendingTask {
    channel: String,
    chat_id: String,
    #[serde(default)]
    conversation_id: String,
}

fn load_pending(task_id: &str) -> Option<PendingTask> {
    let bucket = wasi::keyvalue::store::open("mycelium-channel-pending").ok()?;
    let bytes = bucket.get(task_id).ok().flatten()?;
    serde_json::from_slice(&bytes).ok()
}

fn delete_pending(task_id: &str) -> Result<(), String> {
    let bucket = wasi::keyvalue::store::open("mycelium-channel-pending")
        .map_err(|e| format!("{e:?}"))?;
    bucket.delete(task_id).map_err(|e| format!("{e:?}"))
}

fn set_bot_short_description(token: &str, text: &str) -> Result<(), String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
    let body = serde_json::json!({"short_description": text});
    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let headers = Fields::new();
    headers
        .set("content-type", &[b"application/json".to_vec()])
        .map_err(|e| format!("{e:?}"))?;
    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Post).map_err(|_| "method".to_string())?;
    req.set_scheme(Some(&Scheme::Https)).map_err(|_| "scheme".to_string())?;
    req.set_authority(Some("api.telegram.org"))
        .map_err(|_| "authority".to_string())?;
    req.set_path_with_query(Some(&format!("/bot{token}/setMyShortDescription")))
        .map_err(|_| "path".to_string())?;
    let outgoing = req.body().map_err(|_| "body".to_string())?;
    {
        let stream = outgoing.write().map_err(|_| "stream".to_string())?;
        for chunk in body_bytes.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    OutgoingBody::finish(outgoing, None).map_err(|e| format!("{e:?}"))?;
    let fut = outgoing_handler::handle(req, None).map_err(|e| format!("{e:?}"))?;
    fut.subscribe().block();
    let _ = fut
        .get()
        .ok_or("no resp")?
        .map_err(|_| "consumed".to_string())?
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

fn maybe_update_bot_status(token: &str, rpm_used: u64, rpm_cap: u64, rpd_used: u64, rpd_cap: u64) {
    // Telegram rate-limits setMyShortDescription to ~1/min.
    let Ok(bucket) = wasi::keyvalue::store::open("mycelium-task-state") else { return };
    let now = wasi::clocks::wall_clock::now().seconds;
    let last = bucket
        .get("bot-status/last")
        .ok()
        .flatten()
        .and_then(|b| std::str::from_utf8(&b).ok().map(|s| s.to_string()))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if now < last + 65 {
        return;
    }
    let txt = format!("today: {rpd_used}/{rpd_cap} calls • now: {rpm_used}/{rpm_cap} this minute");
    if set_bot_short_description(token, &txt).is_ok() {
        let _ = bucket.set("bot-status/last", now.to_string().as_bytes());
    }
}

fn progress_edit(state: &TaskStateJson, text: &str) {
    if state.progress_message_id == 0 || state.chat_id.is_empty() {
        return;
    }
    let Some(token) = cfg("telegram.bot_token") else { return };
    let _ = telegram_edit(&token, &state.chat_id, state.progress_message_id, text);
}

fn read_counter(bucket: &wasi::keyvalue::store::Bucket, key: &str) -> u64 {
    bucket
        .get(key)
        .ok()
        .flatten()
        .and_then(|b| std::str::from_utf8(&b).ok().map(|s| s.to_string()))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn quota_snapshot() -> (u64, u64, u64, u64) {
    let now = wasi::clocks::wall_clock::now().seconds;
    let minute = now / 60;
    let day = now / 86_400;
    let rpm_cap = cfg("llm.rpm").and_then(|s| s.parse::<u64>().ok()).unwrap_or(10);
    let rpd_cap = cfg("llm.rpd").and_then(|s| s.parse::<u64>().ok()).unwrap_or(200);
    let Ok(bucket) = wasi::keyvalue::store::open("mycelium-task-state") else {
        return (0, rpm_cap, 0, rpd_cap);
    };
    let keys = bucket.list_keys(None).map(|r| r.keys).unwrap_or_default();
    let mut rpm_used = 0u64;
    let mut rpd_used = 0u64;
    let min_suffix = format!("/min/{minute}");
    let day_suffix = format!("/day/{day}");
    for k in &keys {
        if k.contains(&min_suffix) {
            rpm_used += read_counter(&bucket, k);
        }
        if k.contains(&day_suffix) {
            rpd_used += read_counter(&bucket, k);
        }
    }
    (rpm_used, rpm_cap, rpd_used, rpd_cap)
}

fn telegram_post(token: &str, method: &str, body: &serde_json::Value) -> Result<Vec<u8>, String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
    let body_bytes = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let headers = Fields::new();
    headers
        .set("content-type", &[b"application/json".to_vec()])
        .map_err(|e| format!("{e:?}"))?;
    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Post).map_err(|_| "method".to_string())?;
    req.set_scheme(Some(&Scheme::Https)).map_err(|_| "scheme".to_string())?;
    req.set_authority(Some("api.telegram.org"))
        .map_err(|_| "authority".to_string())?;
    req.set_path_with_query(Some(&format!("/bot{token}/{method}")))
        .map_err(|_| "path".to_string())?;
    let outgoing = req.body().map_err(|_| "body".to_string())?;
    {
        let stream = outgoing.write().map_err(|_| "stream".to_string())?;
        for chunk in body_bytes.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    OutgoingBody::finish(outgoing, None).map_err(|e| format!("{e:?}"))?;
    let fut = outgoing_handler::handle(req, None).map_err(|e| format!("{e:?}"))?;
    fut.subscribe().block();
    let resp = fut
        .get()
        .ok_or("no resp")?
        .map_err(|_| "consumed".to_string())?
        .map_err(|e| format!("{e:?}"))?;
    let incoming = resp.consume().map_err(|_| "no body".to_string())?;
    let stream = incoming.stream().map_err(|_| "no stream".to_string())?;
    let mut buf = Vec::new();
    loop {
        match stream.blocking_read(8192) {
            Ok(c) if c.is_empty() => break,
            Ok(c) => buf.extend_from_slice(&c),
            Err(_) => break,
        }
    }
    Ok(buf)
}

fn telegram_send_with_id(token: &str, chat_id: &str, text: &str) -> Result<i64, String> {
    let body = serde_json::json!({"chat_id": chat_id, "text": text, "parse_mode": "Markdown"});
    let bytes = telegram_post(token, "sendMessage", &body)?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    parsed
        .pointer("/result/message_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| format!("no result.message_id in: {}", String::from_utf8_lossy(&bytes[..bytes.len().min(200)])))
}

fn telegram_edit(token: &str, chat_id: &str, message_id: i64, text: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": text,
        "parse_mode": "Markdown",
    });
    let _ = telegram_post(token, "editMessageText", &body)?;
    Ok(())
}

fn telegram_send(token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
    let body = serde_json::json!({"chat_id": chat_id, "text": text});
    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let headers = Fields::new();
    headers
        .set("content-type", &[b"application/json".to_vec()])
        .map_err(|e| format!("{e:?}"))?;
    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Post).map_err(|_| "method".to_string())?;
    req.set_scheme(Some(&Scheme::Https)).map_err(|_| "scheme".to_string())?;
    req.set_authority(Some("api.telegram.org"))
        .map_err(|_| "authority".to_string())?;
    req.set_path_with_query(Some(&format!("/bot{token}/sendMessage")))
        .map_err(|_| "path".to_string())?;
    let outgoing = req.body().map_err(|_| "body".to_string())?;
    {
        let stream = outgoing.write().map_err(|_| "stream".to_string())?;
        for chunk in body_bytes.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    OutgoingBody::finish(outgoing, None).map_err(|e| format!("{e:?}"))?;
    let fut = outgoing_handler::handle(req, None).map_err(|e| format!("{e:?}"))?;
    fut.subscribe().block();
    let _resp = fut
        .get()
        .ok_or("no resp")?
        .map_err(|_| "consumed".to_string())?
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

fn publish(subject: &str, body: Vec<u8>) -> Result<(), String> {
    wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
        subject: subject.to_string(),
        reply_to: None,
        body,
    })
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let body = msg.body;
        match msg.subject.as_str() {
            "mycelium.task.submit" => {
                let task: TaskJson =
                    serde_json::from_slice(&body).map_err(|e| format!("decode task: {e}"))?;

                // wash 2.1.0 fans every NATS message to multiple component
                // instances in parallel; without a hard dedup here, each
                // parallel instance sends its own "🤔 thinking…" Telegram
                // placeholder. claim_task is a marker-CAS on task-state KV
                // keyed by task_id; first writer wins.
                if !claim_task(&task.id) {
                    return Ok(());
                }

                // Look up chat_id from the pending entry (channel-router wrote it
                // when the inbound raw arrived). Send a "thinking" placeholder
                // and store its message_id so subsequent edits can update it.
                let (chat_id, progress_message_id) = match load_pending(&task.id) {
                    Some(p) if p.channel == "telegram" => {
                        let mid = cfg("telegram.bot_token")
                            .and_then(|tok| telegram_send_with_id(&tok, &p.chat_id, "🤔 thinking…").ok())
                            .unwrap_or(0);
                        (p.chat_id, mid)
                    }
                    _ => (String::new(), 0i64),
                };

                let state = TaskStateJson {
                    task: task.clone(),
                    status: "running".into(),
                    output: None,
                    error: None,
                    step: 0,
                    pending_tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                    progress_message_id,
                    chat_id,
                    // max_steps gets refined later if we can read AgentConfig;
                    // 4 is the safe default that matches the previous hardcoded cap.
                    max_steps: 4,
                };
                save_state(&state)?;

                let step = StepRequest {
                    task_id: &task.id,
                    conversation_id: &task.conversation_id,
                    agent_id: &task.agent_id,
                };
                let step_body = serde_json::to_vec(&step).map_err(|e| e.to_string())?;
                publish("mycelium.task.step.agent", step_body)?;
                Ok(())
            }
            "mycelium.step.tool-calls" => {
                let req: serde_json::Value =
                    serde_json::from_slice(&body).map_err(|e| format!("decode tc: {e}"))?;
                let task_id = req
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or("no task_id")?
                    .to_string();
                let Some(mut state) = load_state(&task_id)? else { return Ok(()) };
                // max-step guard. Stored agent.max_steps would be authoritative;
                // for now hardcode 4 to match the default.
                if state.step >= 4 {
                    state.status = "failed".into();
                    state.error = Some("max steps exceeded".into());
                    save_state(&state)?;
                    let res = serde_json::json!({
                        "task_id": task_id,
                        "output": null,
                        "error": "max steps exceeded",
                    });
                    publish("mycelium.step.result", serde_json::to_vec(&res).map_err(|e| e.to_string())?)?;
                    return Ok(());
                }
                let calls_v = req.get("calls").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let mut calls = Vec::new();
                for c in &calls_v {
                    let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let arguments = c.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}").to_string();
                    if !name.is_empty() {
                        calls.push(ToolCallSpec { id, name, arguments });
                    }
                }
                state.pending_tool_calls = calls.clone();
                state.tool_results.clear();
                state.status = "waiting_tools".into();
                state.step += 1;
                let max = if state.max_steps > 0 { state.max_steps } else { 4 };
                let names: Vec<String> = calls.iter().map(|c| c.name.clone()).collect();
                let label = if names.len() == 1 {
                    format!("🔧 {}({})", names[0], &calls[0].arguments[..calls[0].arguments.len().min(80)])
                } else {
                    format!("🔧 calling {} tools: {}", names.len(), names.join(", "))
                };
                progress_edit(&state, &format!("{label}\n_step {}/{max}_", state.step));
                save_state(&state)?;
                for c in &calls {
                    let payload = serde_json::json!({
                        "task_id": task_id,
                        "call_id": c.id,
                        "arguments": c.arguments,
                    });
                    publish(
                        &format!("mycelium.tool.call.{}", c.name),
                        serde_json::to_vec(&payload).map_err(|e| e.to_string())?,
                    )?;
                }
                Ok(())
            }
            "mycelium.tool.result" => {
                let req: serde_json::Value =
                    serde_json::from_slice(&body).map_err(|e| format!("decode tr: {e}"))?;
                let task_id = req
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or("no task_id")?
                    .to_string();
                let call_id = req
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let Some(mut state) = load_state(&task_id)? else { return Ok(()) };
                // Look up which tool produced this so we can label the message.
                let name = state
                    .pending_tool_calls
                    .iter()
                    .find(|p| p.id == call_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                let output = req.get("output").and_then(|v| v.as_str()).map(|s| s.to_string());
                let error = req.get("error").and_then(|v| v.as_str()).map(|s| s.to_string());
                state.tool_results.push(ToolResultRec { id: call_id, name: name.clone(), output: output.clone(), error: error.clone() });
                let done = state.tool_results.len() >= state.pending_tool_calls.len();
                let max = if state.max_steps > 0 { state.max_steps } else { 4 };
                let preview = match (&output, &error) {
                    (Some(o), _) => {
                        let s = o.replace('\n', " ");
                        format!("{} → {}", name, &s[..s.len().min(80)])
                    }
                    (_, Some(e)) => format!("{name} → error: {}", &e[..e.len().min(80)]),
                    _ => format!("{name} → (empty)"),
                };
                if done {
                    progress_edit(&state, &format!("📊 {preview}\n_step {}/{max} • thinking…_", state.step));
                } else {
                    let remaining = state.pending_tool_calls.len() - state.tool_results.len();
                    progress_edit(&state, &format!("📊 {preview}\n_waiting for {remaining} more tools…_"));
                }
                save_state(&state)?;
                if !done {
                    return Ok(());
                }
                // All results in — fold them into conversation as tool messages,
                // then re-arm the agent step.
                // Gemini's OpenAI-compat surface rejects Tool-role messages
                // without a matching prior assistant.tool_calls entry, which we
                // don't reconstruct. Fold the tool result into a synthetic User
                // message instead — it goes straight into the prompt so the
                // model sees the data on the next pass.
                if let Some(pending) = load_pending(&task_id) {
                    if !pending.conversation_id.is_empty() {
                        for r in &state.tool_results {
                            let content = match (&r.output, &r.error) {
                                (Some(o), _) => format!("[tool {} returned: {}]", r.name, o),
                                (_, Some(e)) => format!("[tool {} failed: {}]", r.name, e),
                                _ => format!("[tool {} returned: <empty>]", r.name),
                            };
                            let _ = mycelium::conversation::conversations::append_message(
                                &pending.conversation_id,
                                mycelium::types::types::MessageRole::User,
                                &content,
                                None,
                            );
                        }
                    }
                }
                // Clear pending; tool_results cleared on next tool-calls.
                state.pending_tool_calls.clear();
                state.status = "running".into();
                save_state(&state)?;
                // Reset the agent's dedup claim so the next step.agent fan-out
                // is allowed to call Gemini exactly once.
                if let Ok(b) = wasi::keyvalue::store::open("mycelium-task-state") {
                    let _ = b.delete(&format!("agent-claim/{task_id}"));
                }
                let step = StepRequest {
                    task_id: &task_id,
                    conversation_id: &state.task.conversation_id,
                    agent_id: &state.task.agent_id,
                };
                publish(
                    "mycelium.task.step.agent",
                    serde_json::to_vec(&step).map_err(|e| e.to_string())?,
                )?;
                Ok(())
            }
            "mycelium.step.result" => {
                let res: StepResult =
                    serde_json::from_slice(&body).map_err(|e| format!("decode step: {e}"))?;
                let reply_text = res
                    .output
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| res.error.clone().map(|e| format!("(error: {e})")));
                if let Some(mut state) = load_state(&res.task_id)? {
                    state.status = if res.error.is_some() {
                        "failed"
                    } else {
                        "done"
                    }
                    .into();
                    state.output = res.output;
                    state.error = res.error;
                    save_state(&state)?;
                }
                if let Some(text) = reply_text {
                    if let Some(pending) = load_pending(&res.task_id) {
                        // Persist the assistant reply so the next user message
                        // sees it in conversation history.
                        if !pending.conversation_id.is_empty() {
                            let _ = mycelium::conversation::conversations::append_message(
                                &pending.conversation_id,
                                mycelium::types::types::MessageRole::Assistant,
                                &text,
                                None,
                            );
                        }
                        if pending.channel == "telegram" {
                            if let Some(token) = cfg("telegram.bot_token") {
                                let (rpm_used, rpm_cap, rpd_used, rpd_cap) = quota_snapshot();
                                let max = if let Some(st) = load_state(&res.task_id).ok().flatten() { st.max_steps.max(1) } else { 4 };
                                let step = load_state(&res.task_id).ok().flatten().map(|s| s.step).unwrap_or(0);
                                let footer = format!(
                                    "_step {step}/{max} • RPM {rpm_used}/{rpm_cap} • RPD {rpd_used}/{rpd_cap}_"
                                );
                                let body = format!("{text}\n\n{footer}");
                                // Edit the progress placeholder if we have it,
                                // otherwise fall back to a fresh send.
                                let edited = load_state(&res.task_id).ok().flatten()
                                    .filter(|s| s.progress_message_id != 0 && !s.chat_id.is_empty())
                                    .and_then(|s| telegram_edit(&token, &s.chat_id, s.progress_message_id, &body).ok());
                                if edited.is_none() {
                                    let _ = telegram_send(&token, &pending.chat_id, &body);
                                }
                                maybe_update_bot_status(
                                    &token, rpm_used, rpm_cap, rpd_used, rpd_cap,
                                );
                            }
                        }
                        let _ = delete_pending(&res.task_id);
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

export!(Component);
