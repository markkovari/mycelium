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

fn call_llm(
    model: &str,
    endpoint: &str,
    api_key: Option<String>,
    prompt: &str,
) -> Result<String, String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest};

    let url = parse_url(endpoint)?;

    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
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

    let parsed: Value = serde_json::from_slice(&buf).map_err(|e| e.to_string())?;
    let content = parsed
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(content)
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
                let prompt = format!("{system_prompt}\n\n(conversation {})", req.conversation_id);
                let (output, error) = match call_llm(&model, &endpoint, api_key, &prompt) {
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
