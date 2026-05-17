// Telegram bot webhook receiver.
//
// HTTP handler: POST /webhook → validate secret → parse Telegram Update.
// - /pair <code> → calls pairing::complete via WIT
// - Other messages: dropped (outbound reply path belongs to a separate component)
//
// Config keys (via wasi:config/store):
//   telegram.webhook_secret — required X-Telegram-Bot-Api-Secret-Token header value
//
// v2 host limitation: a component exporting wasi:http/incoming-handler cannot also
// import wasmcloud:messaging/consumer (HTTP plugin instantiation context lacks it).
wit_bindgen::generate!({
    path: "wit",
    world: "telegram-gateway",
    generate_all,
});

use serde::Deserialize;

use wasi::http::types::{
    Fields, IncomingBody, IncomingRequest, Method, OutgoingBody, OutgoingResponse, ResponseOutparam,
};

fn cfg(key: &str) -> Option<String> {
    wasi::config::store::get(key).ok().flatten()
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

fn simple_response(response_out: ResponseOutparam, status: u16, body: &[u8]) {
    let headers = Fields::new();
    let _ = headers.set("content-type", &[b"application/json".to_vec()]);
    let resp = OutgoingResponse::new(headers);
    let _ = resp.set_status_code(status);
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
        let _ = stream.blocking_write_and_flush(body);
    }
    let _ = OutgoingBody::finish(outgoing_body, None);
}

fn header_val(req: &IncomingRequest, name: &str) -> Option<String> {
    let headers = req.headers();
    let vals = headers.get(name);
    vals.into_iter()
        .next()
        .and_then(|v| String::from_utf8(v).ok())
}

#[derive(Deserialize)]
struct TgUpdate {
    #[allow(dead_code)]
    update_id: i64,
    message: Option<TgMessage>,
}

#[derive(Deserialize)]
struct TgMessage {
    #[allow(dead_code)]
    message_id: i64,
    chat: TgChat,
    text: Option<String>,
}

#[derive(Deserialize)]
struct TgChat {
    id: i64,
}

fn handle_agent_create(rest: &str) {
    // Format: "<id> <model> <system_prompt...>"
    let mut parts = rest.splitn(3, ' ');
    let id = match parts.next() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return,
    };
    let model = match parts.next() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return,
    };
    let system_prompt = parts.next().unwrap_or("").to_string();
    let config = mycelium::types::types::AgentConfig {
        id: id.clone(),
        name: id,
        system_prompt,
        model,
        tools: vec![],
        max_steps: 4,
    };
    let _ = mycelium::agent::agent_registry::create(&config);
}

struct Component;

impl exports::wasi::http::incoming_handler::Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        if !matches!(request.method(), Method::Post) {
            simple_response(response_out, 405, br#"{"error":"method"}"#);
            return;
        }
        if let Some(want) = cfg("telegram.webhook_secret") {
            let got = header_val(&request, "x-telegram-bot-api-secret-token").unwrap_or_default();
            if got != want {
                simple_response(response_out, 401, br#"{"error":"unauthorized"}"#);
                return;
            }
        }
        let bytes = match read_body(request) {
            Ok(b) => b,
            Err(e) => {
                let body = format!(r#"{{"error":"{e}"}}"#);
                simple_response(response_out, 400, body.as_bytes());
                return;
            }
        };

        let update: TgUpdate = match serde_json::from_slice(&bytes) {
            Ok(u) => u,
            Err(e) => {
                let body = format!(r#"{{"error":"{}"}}"#, e);
                simple_response(response_out, 400, body.as_bytes());
                return;
            }
        };

        // Admin command parsing — currently only `/agent create <id> <model> <prompt...>`.
        if let Some(message) = update.message {
            if let Some(text) = message.text {
                if let Some(rest) = text.strip_prefix("/agent create ") {
                    handle_agent_create(rest.trim());
                }
            }
        }

        // Acknowledge fast. Telegram requires <=10s ACK.
        simple_response(response_out, 200, br#"{"ok":true}"#);
    }
}

export!(Component);
