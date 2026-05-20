// LLM step executor.
// Subscribes to mycelium.task.step.agent.
// Calls the configured LLM via wasi:http/outgoing-handler (OpenAI-compatible /v1/chat/completions).
// Publishes final result to mycelium.step.result.
//
// Config keys (via wasi:config/runtime):
//   llm.endpoint    — default http://host.docker.internal:11434/v1/chat/completions (Ollama)
//   llm.model       — default qwen2.5:0.5b
//   llm.api_key     — optional bearer token
wit_bindgen::generate!({
    path: "wit",
    world: "agent",
    generate_all,
});

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize)]
struct StepReq {
    task_id: String,
    conversation_id: String,
    agent_id: String,
}

#[derive(Serialize)]
struct StepResult {
    task_id: String,
    output: Option<String>,
    error: Option<String>,
}

fn cfg(key: &str, default: &str) -> String {
    match wasi::config::store::get(key) {
        Ok(Some(v)) => v,
        _ => default.to_string(),
    }
}

#[derive(Deserialize)]
struct StoredAgentConfig {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    name: String,
    system_prompt: String,
    model: String,
    #[allow(dead_code)]
    #[serde(default)]
    tools: Vec<String>,
    #[allow(dead_code)]
    #[serde(default)]
    max_steps: u32,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}

fn load_agent_config(agent_id: &str) -> Option<StoredAgentConfig> {
    let bucket = wasi::keyvalue::store::open("mycelium-agent-config").ok()?;
    let bytes = bucket.get(&format!("agent/{agent_id}")).ok().flatten()?;
    serde_json::from_slice(&bytes).ok()
}

const RATE_BUCKET: &str = "mycelium-task-state";
const DEFAULT_RPM: u64 = 10;
const DEFAULT_RPD: u64 = 200;

/// Per-minute cap. wasi:config keys: llm.rpm (0 disables).
fn rpm_check(agent_id: &str) -> Result<u64, u64> {
    let rpm: u64 = cfg("llm.rpm", &DEFAULT_RPM.to_string())
        .parse()
        .unwrap_or(DEFAULT_RPM);
    if rpm == 0 {
        return Ok(u64::MAX);
    }
    let now = wasi::clocks::wall_clock::now();
    let minute = now.seconds / 60;
    let key = format!("rate/{agent_id}/min/{minute}");
    let bucket = match wasi::keyvalue::store::open(RATE_BUCKET) {
        Ok(b) => b,
        Err(_) => return Ok(rpm),
    };
    let count = wasi::keyvalue::atomics::increment(&bucket, &key, 1).unwrap_or(0);
    if count <= rpm {
        Ok(rpm - count)
    } else {
        Err(count - rpm)
    }
}

/// Per-day cap. wasi:config keys: llm.rpd (0 disables).
fn rpd_check(agent_id: &str) -> Result<u64, u64> {
    let rpd: u64 = cfg("llm.rpd", &DEFAULT_RPD.to_string())
        .parse()
        .unwrap_or(DEFAULT_RPD);
    if rpd == 0 {
        return Ok(u64::MAX);
    }
    let now = wasi::clocks::wall_clock::now();
    let day = now.seconds / 86_400;
    let key = format!("rate/{agent_id}/day/{day}");
    let bucket = match wasi::keyvalue::store::open(RATE_BUCKET) {
        Ok(b) => b,
        Err(_) => return Ok(rpd),
    };
    let count = wasi::keyvalue::atomics::increment(&bucket, &key, 1).unwrap_or(0);
    if count <= rpd {
        Ok(rpd - count)
    } else {
        Err(count - rpd)
    }
}


const CIRCUIT_OPEN_UNTIL_KEY: &str = "circuit/open-until";
const CIRCUIT_FAILS_KEY: &str = "circuit/fails";
const CIRCUIT_FAIL_THRESHOLD: u64 = 5;
const CIRCUIT_COOLDOWN_S: u64 = 60;

fn circuit_open() -> Option<u64> {
    let bucket = wasi::keyvalue::store::open(RATE_BUCKET).ok()?;
    let bytes = bucket.get(CIRCUIT_OPEN_UNTIL_KEY).ok().flatten()?;
    let until: u64 = String::from_utf8(bytes).ok()?.parse().ok()?;
    let now = wasi::clocks::wall_clock::now().seconds;
    if until > now { Some(until - now) } else { None }
}

fn note_circuit_failure() {
    let Ok(bucket) = wasi::keyvalue::store::open(RATE_BUCKET) else { return };
    let n = wasi::keyvalue::atomics::increment(&bucket, CIRCUIT_FAILS_KEY, 1).unwrap_or(0);
    if n >= CIRCUIT_FAIL_THRESHOLD {
        let now = wasi::clocks::wall_clock::now().seconds;
        let until = now + CIRCUIT_COOLDOWN_S;
        let _ = bucket.set(CIRCUIT_OPEN_UNTIL_KEY, until.to_string().as_bytes());
        let _ = bucket.set(CIRCUIT_FAILS_KEY, b"0");
    }
}

fn note_circuit_success() {
    if let Ok(bucket) = wasi::keyvalue::store::open(RATE_BUCKET) {
        let _ = bucket.set(CIRCUIT_FAILS_KEY, b"0");
    }
}

/// Atomic per-step claim. Each NEW step.agent invocation for a task should
/// produce a fresh increment from the executor's perspective; the agent
/// itself only needs to know that one of the parallel runs wins. We dedup
/// by (task_id, current step value in task-state) — but to keep things
/// simple, we just dedup on task_id + a coarse "claim" counter that the
/// executor resets per step.
fn claim_step(task_id: &str) -> bool {
    let Ok(bucket) = wasi::keyvalue::store::open(RATE_BUCKET) else { return true };
    let key = format!("agent-claim/{task_id}");
    let n = wasi::keyvalue::atomics::increment(&bucket, &key, 1).unwrap_or(0);
    // Only the first claimer per step proceeds. Executor resets the key when
    // it pushes the next step.agent (after collecting tool results).
    n == 1
}

/// For each name in `wanted`, look up its JSON schema in `mycelium-tools`
/// KV and produce an OpenAI tools[] entry. Missing entries are skipped.
fn load_tool_specs(wanted: &[String]) -> Vec<Value> {
    if wanted.is_empty() {
        return Vec::new();
    }
    let Ok(bucket) = wasi::keyvalue::store::open("mycelium-tools") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for name in wanted {
        let Ok(Some(bytes)) = bucket.get(name) else { continue };
        let Ok(spec) = serde_json::from_slice::<Value>(&bytes) else { continue };
        let description = spec.get("description").cloned().unwrap_or_else(|| json!(""));
        let parameters = spec.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object"}));
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

fn publish(subject: &str, body: Vec<u8>) -> Result<(), String> {
    wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
        subject: subject.to_string(),
        reply_to: None,
        body,
    })
}

struct ParsedUrl {
    scheme: wasi::http::types::Scheme,
    authority: String,
    path_and_query: String,
}

fn parse_url(s: &str) -> Result<ParsedUrl, String> {
    let (scheme, rest) = if let Some(r) = s.strip_prefix("http://") {
        (wasi::http::types::Scheme::Http, r)
    } else if let Some(r) = s.strip_prefix("https://") {
        (wasi::http::types::Scheme::Https, r)
    } else {
        return Err(format!("unknown scheme: {s}"));
    };
    let (authority, path_and_query) = match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.to_string(), "/".to_string()),
    };
    Ok(ParsedUrl {
        scheme,
        authority,
        path_and_query,
    })
}

fn do_request(
    model: &str,
    url: &ParsedUrl,
    api_key: Option<&str>,
    messages: &[Value],
    tools: &[Value],
) -> Result<(u16, Vec<u8>), String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest};

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
        body["tool_choice"] = json!("auto");
    }
    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

    let headers = Fields::new();
    headers
        .set("content-type", &[b"application/json".to_vec()])
        .map_err(|e| format!("{e:?}"))?;
    if let Some(key) = api_key {
        let auth = format!("Bearer {key}");
        let _ = headers.set("authorization", &[auth.into_bytes()]);
    }

    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Post)
        .map_err(|_| "set method".to_string())?;
    req.set_scheme(Some(&url.scheme))
        .map_err(|_| "set scheme".to_string())?;
    req.set_authority(Some(&url.authority))
        .map_err(|_| "set authority".to_string())?;
    req.set_path_with_query(Some(&url.path_and_query))
        .map_err(|_| "set path".to_string())?;

    let outgoing_body = req.body().map_err(|_| "no outgoing body".to_string())?;
    {
        let stream = outgoing_body
            .write()
            .map_err(|_| "no body write stream".to_string())?;
        for chunk in body_bytes.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .map_err(|e| format!("body write: {e:?}"))?;
        }
    }
    OutgoingBody::finish(outgoing_body, None).map_err(|e| format!("finish body: {e:?}"))?;

    let future_resp = outgoing_handler::handle(req, None).map_err(|e| format!("handle: {e:?}"))?;
    future_resp.subscribe().block();
    let resp = future_resp
        .get()
        .ok_or("response missing".to_string())?
        .map_err(|_| "response already consumed".to_string())?
        .map_err(|e| format!("recv: {e:?}"))?;

    let status = resp.status();
    let incoming_body = resp.consume().map_err(|_| "no body".to_string())?;
    let stream = incoming_body
        .stream()
        .map_err(|_| "no body stream".to_string())?;
    let mut buf = Vec::new();
    loop {
        match stream.blocking_read(8192) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(stream);
    Ok((status, buf))
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

fn sleep_seconds(s: u64) {
    let ns = s.saturating_mul(1_000_000_000);
    let p = wasi::clocks::monotonic_clock::subscribe_duration(ns);
    p.block();
}

/// One assistant turn from the LLM. Either a plain text reply or a set of
/// tool calls the agent should dispatch.
pub enum LlmReply {
    Text(String),
    Tools(Vec<ToolCall>),
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String, // raw JSON string as Gemini/OpenAI emit it
}

fn call_llm(
    model: &str,
    endpoint: &str,
    api_key: Option<String>,
    messages: &[Value],
    tools: &[Value],
) -> Result<LlmReply, String> {
    const MAX_ATTEMPTS: u32 = 4;
    const MAX_DELAY_S: u64 = 60;
    let url = parse_url(endpoint)?;
    let key = api_key.as_deref();
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let (status, body) = do_request(model, &url, key, messages, tools)?;
        if status == 200 {
            let parsed: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
            // Prefer tool_calls if present.
            if let Some(arr) = parsed
                .pointer("/choices/0/message/tool_calls")
                .and_then(|v| v.as_array())
            {
                let mut calls = Vec::new();
                for c in arr {
                    let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let name = c
                        .pointer("/function/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = c
                        .pointer("/function/arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    if !name.is_empty() {
                        calls.push(ToolCall { id, name, arguments });
                    }
                }
                if !calls.is_empty() {
                    return Ok(LlmReply::Tools(calls));
                }
            }
            let content = parsed
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return Ok(LlmReply::Text(content));
        }
        let retryable = status == 429 || (500..600).contains(&status);
        if retryable && attempt < MAX_ATTEMPTS {
            let delay = parse_retry_delay_seconds(&body)
                .unwrap_or_else(|| 1u64 << (attempt - 1))
                .min(MAX_DELAY_S);
            sleep_seconds(delay);
            continue;
        }
        let snippet = String::from_utf8_lossy(&body);
        return Err(format!("HTTP {status}: {}", &snippet[..snippet.len().min(400)]));
    }
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let body = msg.body;
        match msg.subject.as_str() {
            "mycelium.task.step.agent" => {
                let req: StepReq = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
                // Dedup concurrent invocations of the same step. wash 2.1.0
                // fans the message to multiple instances of this workload
                // simultaneously; without this they all call Gemini for the
                // same task.
                if !claim_step(&req.task_id) {
                    return Ok(());
                }
                let stored = load_agent_config(&req.agent_id);
                let endpoint = stored
                    .as_ref()
                    .and_then(|s| s.endpoint.clone())
                    .unwrap_or_else(|| {
                        cfg(
                            "llm.endpoint",
                            "http://host.docker.internal:11434/v1/chat/completions",
                        )
                    });
                let model = stored
                    .as_ref()
                    .map(|s| s.model.clone())
                    .unwrap_or_else(|| cfg("llm.model", "qwen2.5:0.5b"));
                let api_key = stored
                    .as_ref()
                    .and_then(|s| s.api_key.clone())
                    .or_else(|| wasi::config::store::get("llm.api_key").ok().flatten());
                let system_prompt = stored
                    .as_ref()
                    .map(|s| s.system_prompt.clone())
                    .unwrap_or_else(|| format!("You are mycelium agent {}.", req.agent_id));

                // Build OpenAI-style message array: system prompt + full history.
                let mut messages = vec![json!({"role": "system", "content": system_prompt})];
                match mycelium::conversation::conversations::get_messages(&req.conversation_id) {
                    Ok(history) => {
                        for m in history {
                            let role = match m.role {
                                mycelium::types::types::MessageRole::System => "system",
                                mycelium::types::types::MessageRole::User => "user",
                                mycelium::types::types::MessageRole::Assistant => "assistant",
                                mycelium::types::types::MessageRole::Tool => "tool",
                            };
                            messages.push(json!({"role": role, "content": m.content}));
                        }
                    }
                    Err(e) => {
                        wasi::logging::logging::log(
                            wasi::logging::logging::Level::Warn,
                            "agent",
                            &format!("get_messages failed: {e:?}"),
                        );
                    }
                };
                // Circuit breaker: refuse fast if the breaker is open.
                if let Some(cooldown) = circuit_open() {
                    let result = StepResult {
                        task_id: req.task_id,
                        output: None,
                        error: Some(format!(
                            "circuit open; retry in {cooldown}s after repeated LLM failures"
                        )),
                    };
                    publish(
                        "mycelium.step.result",
                        serde_json::to_vec(&result).map_err(|e| e.to_string())?,
                    )?;
                    return Ok(());
                }

                // Per-day cap (hard ceiling on paid-key cost).
                if let Err(over) = rpd_check(&req.agent_id) {
                    let result = StepResult {
                        task_id: req.task_id,
                        output: None,
                        error: Some(format!(
                            "daily LLM budget exceeded ({over} over cap); resets at UTC midnight"
                        )),
                    };
                    publish(
                        "mycelium.step.result",
                        serde_json::to_vec(&result).map_err(|e| e.to_string())?,
                    )?;
                    return Ok(());
                }
                // Per-minute cap (smooths bursts).
                if let Err(over) = rpm_check(&req.agent_id) {
                    let result = StepResult {
                        task_id: req.task_id,
                        output: None,
                        error: Some(format!(
                            "rate limit hit ({over} over budget this minute); try again shortly"
                        )),
                    };
                    publish(
                        "mycelium.step.result",
                        serde_json::to_vec(&result).map_err(|e| e.to_string())?,
                    )?;
                    return Ok(());
                }

                // Build tools array for the LLM from agent.tools (names) +
                // mycelium-tools KV (schema for each name).
                let tool_names = stored.as_ref().map(|s| s.tools.clone()).unwrap_or_default();
                let tool_specs = load_tool_specs(&tool_names);

                let llm_outcome = call_llm(&model, &endpoint, api_key, &messages, &tool_specs);
                match &llm_outcome {
                    Ok(_) => note_circuit_success(),
                    Err(_) => note_circuit_failure(),
                }
                let (out, err, tool_calls) = match llm_outcome {
                    Ok(LlmReply::Text(content)) => (Some(content), None, Vec::new()),
                    Ok(LlmReply::Tools(calls)) => (None, None, calls),
                    Err(e) => (None, Some(e), Vec::new()),
                };

                if !tool_calls.is_empty() {
                    let payload = json!({
                        "task_id": req.task_id,
                        "conversation_id": req.conversation_id,
                        "agent_id": req.agent_id,
                        "calls": tool_calls,
                    });
                    publish(
                        "mycelium.step.tool-calls",
                        serde_json::to_vec(&payload).map_err(|e| e.to_string())?,
                    )?;
                    return Ok(());
                }

                let result = StepResult {
                    task_id: req.task_id,
                    output: out,
                    error: err,
                };
                publish(
                    "mycelium.step.result",
                    serde_json::to_vec(&result).map_err(|e| e.to_string())?,
                )?;
                Ok(())
            }
            "mycelium.tool.result" => Ok(()),
            _ => Ok(()),
        }
    }
}

export!(Component);
