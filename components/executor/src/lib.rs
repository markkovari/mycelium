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

fn read_counter(bucket: &wasi::keyvalue::store::Bucket, key: &str) -> u64 {
    bucket
        .get(key)
        .ok()
        .flatten()
        .and_then(|b| std::str::from_utf8(&b).ok().map(|s| s.to_string()))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn quota_footer() -> String {
    let now = wasi::clocks::wall_clock::now().seconds;
    let minute = now / 60;
    let day = now / 86_400;
    let Ok(bucket) = wasi::keyvalue::store::open("mycelium-task-state") else { return String::new() };
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
    let rpm_cap = cfg("llm.rpm").and_then(|s| s.parse::<u64>().ok()).unwrap_or(10);
    let rpd_cap = cfg("llm.rpd").and_then(|s| s.parse::<u64>().ok()).unwrap_or(200);
    format!("_RPM {rpm_used}/{rpm_cap} • RPD {rpd_used}/{rpd_cap}_")
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

                let state = TaskStateJson {
                    task: task.clone(),
                    status: "running".into(),
                    output: None,
                    error: None,
                    step: 0,
                    pending_tool_calls: Vec::new(),
                    tool_results: Vec::new(),
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
                state.tool_results.push(ToolResultRec { id: call_id, name, output, error });
                let done = state.tool_results.len() >= state.pending_tool_calls.len();
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
                                let footer = quota_footer();
                                let body = if footer.is_empty() {
                                    text.clone()
                                } else {
                                    format!("{text}\n\n{footer}")
                                };
                                let _ = telegram_send(&token, &pending.chat_id, &body);
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
