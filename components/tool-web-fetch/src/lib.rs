// web_fetch tool: GET an HTTPS URL, return body truncated to 4 KB.
// Subscribe: mycelium.tool.call.web_fetch
// Reply:     mycelium.tool.result
//
// Input: arguments = {"url": "https://example.com"}
wit_bindgen::generate!({
    path: "wit",
    world: "tool-web-fetch",
    generate_all,
});

use serde_json::{json, Value};

fn publish(subject: &str, body: Vec<u8>) {
    let _ = wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
        subject: subject.to_string(),
        reply_to: None,
        body,
    });
}

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
        .set("user-agent", &[b"mycelium-tool-web-fetch/0.1".to_vec()])
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

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        if msg.subject != "mycelium.tool.call.web_fetch" {
            return Ok(());
        }
        let req: Value = serde_json::from_slice(&msg.body).map_err(|e| e.to_string())?;
        let task_id = req.get("task_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let call_id = req.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let args_raw = req.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
        let args: Value = serde_json::from_str(args_raw).unwrap_or_else(|_| json!({}));
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");

        let (output, error) = if url.is_empty() {
            (None, Some("missing url argument".to_string()))
        } else {
            match http_get(url) {
                Ok(text) => (Some(text), None),
                Err(e) => (None, Some(e)),
            }
        };

        let out = json!({
            "task_id": task_id,
            "call_id": call_id,
            "output": output,
            "error": error,
        });
        publish(
            "mycelium.tool.result",
            serde_json::to_vec(&out).map_err(|e| e.to_string())?,
        );
        Ok(())
    }
}

export!(Component);
