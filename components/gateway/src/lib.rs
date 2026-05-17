// HTTP ingress component.
//
// Routes:
//   GET  /health                       → 200 OK
//   POST /conversations                → mycelium:conversation/conversations::create
//   GET  /conversations/:id            → fetch single
//   GET  /conversations/:id/messages   → list messages
//   POST /conversations/:id/messages   → append (assistant reply triggered out-of-band)
//
// All responses JSON.
//
// v2 host limitation: HTTP plugin's instantiation context does not expose
// wasmcloud:messaging/consumer. Cross-component fan-out happens via WIT calls only;
// any NATS publish must be done by a non-HTTP component (e.g. conversation-store on
// user-message append, future feature).
wit_bindgen::generate!({
    path: "wit",
    world: "gateway",
    generate_all,
});

use serde::Deserialize;
use serde_json::{json, Value};

use wasi::http::types::{
    Fields, IncomingBody, IncomingRequest, Method, OutgoingBody, OutgoingResponse, ResponseOutparam,
};

#[derive(Deserialize)]
struct CreateConvReq {
    agent_id: String,
    title: Option<String>,
}

#[derive(Deserialize)]
struct AgentReq {
    id: String,
    name: Option<String>,
    system_prompt: Option<String>,
    model: String,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default = "default_max_steps")]
    max_steps: u32,
}

fn default_max_steps() -> u32 {
    4
}

fn agent_to_json(c: &mycelium::types::types::AgentConfig) -> Value {
    json!({
        "id": c.id,
        "name": c.name,
        "system_prompt": c.system_prompt,
        "model": c.model,
        "tools": c.tools,
        "max_steps": c.max_steps,
    })
}

fn agent_from_req(r: AgentReq) -> mycelium::types::types::AgentConfig {
    mycelium::types::types::AgentConfig {
        id: r.id.clone(),
        name: r.name.unwrap_or(r.id),
        system_prompt: r.system_prompt.unwrap_or_default(),
        model: r.model,
        tools: r.tools,
        max_steps: r.max_steps,
    }
}

fn agents_create(req: IncomingRequest, response_out: ResponseOutparam) {
    let bytes = match read_body(req) {
        Ok(b) => b,
        Err(e) => {
            json_response(response_out, 400, json!({"error": e}));
            return;
        }
    };
    let body: AgentReq = match serde_json::from_slice(&bytes) {
        Ok(b) => b,
        Err(e) => {
            json_response(response_out, 400, json!({"error": e.to_string()}));
            return;
        }
    };
    match mycelium::agent::agent_registry::create(&agent_from_req(body)) {
        Ok(c) => json_response(response_out, 201, agent_to_json(&c)),
        Err(e) => json_response(response_out, 500, json!({"error": format!("{e:?}")})),
    }
}

fn agents_get(id: &str, response_out: ResponseOutparam) {
    match mycelium::agent::agent_registry::get(id) {
        Ok(c) => json_response(response_out, 200, agent_to_json(&c)),
        Err(e) => json_response(response_out, 404, json!({"error": format!("{e:?}")})),
    }
}

fn agents_list(response_out: ResponseOutparam) {
    match mycelium::agent::agent_registry::list_agents() {
        Ok(cs) => json_response(
            response_out,
            200,
            json!({ "agents": cs.iter().map(agent_to_json).collect::<Vec<_>>() }),
        ),
        Err(e) => json_response(response_out, 500, json!({"error": format!("{e:?}")})),
    }
}

fn agents_update(id: &str, req: IncomingRequest, response_out: ResponseOutparam) {
    let bytes = match read_body(req) {
        Ok(b) => b,
        Err(e) => {
            json_response(response_out, 400, json!({"error": e}));
            return;
        }
    };
    let mut body: AgentReq = match serde_json::from_slice(&bytes) {
        Ok(b) => b,
        Err(e) => {
            json_response(response_out, 400, json!({"error": e.to_string()}));
            return;
        }
    };
    body.id = id.to_string();
    match mycelium::agent::agent_registry::update(&agent_from_req(body)) {
        Ok(c) => json_response(response_out, 200, agent_to_json(&c)),
        Err(e) => json_response(response_out, 500, json!({"error": format!("{e:?}")})),
    }
}

fn agents_delete(id: &str, response_out: ResponseOutparam) {
    match mycelium::agent::agent_registry::delete(id) {
        Ok(()) => json_response(response_out, 204, json!({})),
        Err(e) => json_response(response_out, 500, json!({"error": format!("{e:?}")})),
    }
}

#[derive(Deserialize)]
struct AppendMsgReq {
    role: String,
    content: String,
    tool_use_id: Option<String>,
}

fn role_from(s: &str) -> mycelium::types::types::MessageRole {
    match s {
        "system" => mycelium::types::types::MessageRole::System,
        "assistant" => mycelium::types::types::MessageRole::Assistant,
        "tool" => mycelium::types::types::MessageRole::Tool,
        _ => mycelium::types::types::MessageRole::User,
    }
}

fn conv_to_json(c: &mycelium::types::types::Conversation) -> Value {
    json!({
        "id": c.id,
        "agent_id": c.agent_id,
        "title": c.title,
        "created_at": c.created_at,
        "updated_at": c.updated_at,
    })
}

fn msg_to_json(m: &mycelium::types::types::Message) -> Value {
    let role = match m.role {
        mycelium::types::types::MessageRole::System => "system",
        mycelium::types::types::MessageRole::User => "user",
        mycelium::types::types::MessageRole::Assistant => "assistant",
        mycelium::types::types::MessageRole::Tool => "tool",
    };
    json!({
        "id": m.id,
        "role": role,
        "content": m.content,
        "tool_use_id": m.tool_use_id,
        "created_at": m.created_at,
    })
}

fn read_body(req: IncomingRequest) -> Result<Vec<u8>, String> {
    let body: IncomingBody = req.consume().map_err(|_| "no body".to_string())?;
    let stream = body.stream().map_err(|_| "no stream".to_string())?;
    let mut buf = Vec::new();
    loop {
        match stream.blocking_read(8192) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(stream);
    Ok(buf)
}

fn json_response(response_out: ResponseOutparam, status: u16, body_val: Value) {
    let headers = Fields::new();
    let _ = headers.set("content-type", &[b"application/json".to_vec()]);
    let resp = OutgoingResponse::new(headers);
    let _ = resp.set_status_code(status);

    let body_bytes = serde_json::to_vec(&body_val).unwrap_or_default();

    let outgoing_body = match resp.body() {
        Ok(b) => b,
        Err(_) => {
            ResponseOutparam::set(
                response_out,
                Err(wasi::http::types::ErrorCode::InternalError(None)),
            );
            return;
        }
    };
    ResponseOutparam::set(response_out, Ok(resp));

    if let Ok(stream) = outgoing_body.write() {
        for chunk in body_bytes.chunks(4096) {
            let _ = stream.blocking_write_and_flush(chunk);
        }
    }
    let _ = OutgoingBody::finish(outgoing_body, None);
}

fn split_path(path: &str) -> Vec<&str> {
    path.trim_start_matches('/').split('/').collect()
}

fn handle_request(req: IncomingRequest, response_out: ResponseOutparam) {
    let method = req.method();
    let path_q = req.path_with_query().unwrap_or_else(|| "/".to_string());
    let (path, _query) = match path_q.find('?') {
        Some(i) => (path_q[..i].to_string(), path_q[i + 1..].to_string()),
        None => (path_q.clone(), String::new()),
    };
    let segs = split_path(&path);

    match (&method, segs.as_slice()) {
        (Method::Get, ["health"]) => {
            json_response(response_out, 200, json!({"status": "ok"}));
        }
        (Method::Post, ["conversations"]) => {
            let bytes = match read_body(req) {
                Ok(b) => b,
                Err(e) => {
                    json_response(response_out, 400, json!({"error": e}));
                    return;
                }
            };
            let body: CreateConvReq = match serde_json::from_slice(&bytes) {
                Ok(b) => b,
                Err(e) => {
                    json_response(response_out, 400, json!({"error": e.to_string()}));
                    return;
                }
            };
            match mycelium::conversation::conversations::create(
                &body.agent_id,
                body.title.as_deref(),
            ) {
                Ok(c) => json_response(response_out, 201, conv_to_json(&c)),
                Err(e) => json_response(response_out, 500, json!({"error": format!("{e:?}")})),
            }
        }
        (Method::Get, ["conversations", id]) => {
            match mycelium::conversation::conversations::get(id) {
                Ok(c) => json_response(response_out, 200, conv_to_json(&c)),
                Err(e) => json_response(response_out, 404, json!({"error": format!("{e:?}")})),
            }
        }
        (Method::Get, ["conversations", id, "messages"]) => {
            match mycelium::conversation::conversations::get_messages(id) {
                Ok(msgs) => json_response(
                    response_out,
                    200,
                    json!({ "messages": msgs.iter().map(msg_to_json).collect::<Vec<_>>() }),
                ),
                Err(e) => json_response(response_out, 500, json!({"error": format!("{e:?}")})),
            }
        }
        (Method::Post, ["conversations", id, "messages"]) => {
            let id = id.to_string();
            let bytes = match read_body(req) {
                Ok(b) => b,
                Err(e) => {
                    json_response(response_out, 400, json!({"error": e}));
                    return;
                }
            };
            let body: AppendMsgReq = match serde_json::from_slice(&bytes) {
                Ok(b) => b,
                Err(e) => {
                    json_response(response_out, 400, json!({"error": e.to_string()}));
                    return;
                }
            };
            let appended = match mycelium::conversation::conversations::append_message(
                &id,
                role_from(&body.role),
                &body.content,
                body.tool_use_id.as_deref(),
            ) {
                Ok(m) => m,
                Err(e) => {
                    json_response(response_out, 500, json!({"error": format!("{e:?}")}));
                    return;
                }
            };
            // v2 host limitation: gateway can't publish to NATS (HTTP plugin context
            // lacks wasmcloud:messaging/consumer). Task submission must come from a
            // non-HTTP component, e.g. an external `nats pub mycelium.task.submit ...`
            // or a future task-scheduler workload that watches new messages.
            json_response(response_out, 201, msg_to_json(&appended));
        }
        (Method::Post, ["agents"]) => agents_create(req, response_out),
        (Method::Get, ["agents"]) => agents_list(response_out),
        (Method::Get, ["agents", id]) => agents_get(id, response_out),
        (Method::Put, ["agents", id]) => agents_update(id, req, response_out),
        (Method::Delete, ["agents", id]) => agents_delete(id, response_out),
        _ => {
            json_response(
                response_out,
                404,
                json!({"error": "not found", "path": path}),
            );
        }
    }
}

struct Component;

impl exports::wasi::http::incoming_handler::Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        handle_request(request, response_out);
    }
}

export!(Component);
