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
) -> Result<(u16, Vec<u8>), String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest};

    let body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });
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

fn call_llm(
    model: &str,
    endpoint: &str,
    api_key: Option<String>,
    messages: &[Value],
) -> Result<String, String> {
    const MAX_ATTEMPTS: u32 = 4;
    const MAX_DELAY_S: u64 = 60;
    let url = parse_url(endpoint)?;
    let key = api_key.as_deref();
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let (status, body) = do_request(model, &url, key, messages)?;
        if status == 200 {
            let parsed: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
            return Ok(parsed
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string());
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
                let (output, error) = match call_llm(&model, &endpoint, api_key, &messages) {
                    Ok(content) => (Some(content), None),
                    Err(e) => (None, Some(e)),
                };
                let result = StepResult {
                    task_id: req.task_id,
                    output,
                    error,
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
