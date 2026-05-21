//! web_fetch skill — GET an HTTP(S) URL, return body truncated to 4 KB.
//!
//! Declares `wasi:http/outgoing-handler` as its capability. The
//! mycelium-tool-runner adds wasi:http to the wasmtime Linker only if
//! the manifest declared it AND the operator allow-list approves it.

wit_bindgen::generate!({
    path: "wit",
    world: "tool-web-fetch",
    generate_all,
});

use serde_json::{json, Value};

use exports::mycelium::tool::tool_provider::Guest;
use mycelium::types::types::{DomainError, ToolCallRequest, ToolCallResult};

struct ParsedUrl {
    scheme: wasi::http::types::Scheme,
    authority: String,
    path_and_query: String,
}

fn parse_url(s: &str) -> Result<ParsedUrl, String> {
    let (scheme, rest) = if let Some(r) = s.strip_prefix("https://") {
        (wasi::http::types::Scheme::Https, r)
    } else if let Some(r) = s.strip_prefix("http://") {
        (wasi::http::types::Scheme::Http, r)
    } else {
        return Err(format!("unknown scheme: {s}"));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.to_string(), "/".to_string()),
    };
    Ok(ParsedUrl {
        scheme,
        authority,
        path_and_query: path,
    })
}

fn http_get(url_str: &str) -> Result<String, String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingRequest};
    let url = parse_url(url_str)?;
    let headers = Fields::new();
    headers
        .set("user-agent", &[b"mycelium-skill-web-fetch/0.1".to_vec()])
        .map_err(|e| format!("{e:?}"))?;
    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Get).map_err(|_| "method".to_string())?;
    req.set_scheme(Some(&url.scheme)).map_err(|_| "scheme".to_string())?;
    req.set_authority(Some(&url.authority)).map_err(|_| "authority".to_string())?;
    req.set_path_with_query(Some(&url.path_and_query)).map_err(|_| "path".to_string())?;
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
    while buf.len() < 4096 {
        match stream.blocking_read(4096) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    Ok(String::from_utf8_lossy(&buf[..buf.len().min(4096)]).to_string())
}

struct Component;

impl Guest for Component {
    fn invoke(call: ToolCallRequest) -> Result<ToolCallResult, DomainError> {
        let args: Value =
            serde_json::from_str(&call.args_json).unwrap_or_else(|_| json!({}));
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() {
            return Ok(ToolCallResult {
                call_id: call.call_id,
                tool_id: call.tool_id,
                output_json: json!({"error": "missing url argument"}).to_string(),
                is_error: true,
            });
        }
        let (output_json, is_error) = match http_get(url) {
            Ok(text) => (json!({"body": text}).to_string(), false),
            Err(e) => (json!({"error": e}).to_string(), true),
        };
        Ok(ToolCallResult {
            call_id: call.call_id,
            tool_id: call.tool_id,
            output_json,
            is_error,
        })
    }
}

export!(Component);
