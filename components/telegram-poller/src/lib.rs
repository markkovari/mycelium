// Telegram transport adapter.
//
// One job: long-poll Telegram and emit every raw update to NATS.
// Subject: `mycelium.channel.telegram.raw`. Payload = the raw `result[i]` JSON.
//
// Self-tick: subscribes `mycelium.telegram.poll.tick`. Each invocation does
// one getUpdates round and re-publishes the tick at the end. Deploy script
// fires the first tick.
//
// Offset persisted in `mycelium-telegram-poller-state` KV (key `telegram/offset`).
//
// Config (wasi:config/store):
//   telegram.bot_token        BotFather token (required to do real work)
//   telegram.poll_timeout_s   Telegram long-poll timeout (default 25)
//   telegram.poll_idle_ms     sleep between empty polls (default 500)
wit_bindgen::generate!({
    path: "wit",
    world: "telegram-poller",
    generate_all,
});

use serde::Deserialize;
use serde_json::Value;

use wasi::http::outgoing_handler;
use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};

const STATE_BUCKET: &str = "mycelium-telegram-poller-state";
const OFFSET_KEY: &str = "telegram/offset";
const TICK_SUBJECT: &str = "mycelium.telegram.poll.tick";
const RAW_SUBJECT: &str = "mycelium.channel.telegram.raw";

fn cfg(key: &str) -> Option<String> {
    wasi::config::store::get(key).ok().flatten()
}

fn sleep_ms(ms: u64) {
    let now = wasi::clocks::monotonic_clock::now();
    let target = now.saturating_add(ms.saturating_mul(1_000_000));
    let pollable = wasi::clocks::monotonic_clock::subscribe_instant(target);
    pollable.block();
}

fn log(level: wasi::logging::logging::Level, msg: &str) {
    wasi::logging::logging::log(level, "telegram-poller", msg);
}

fn read_offset() -> u64 {
    let Ok(bucket) = wasi::keyvalue::store::open(STATE_BUCKET) else {
        return 0;
    };
    bucket
        .get(OFFSET_KEY)
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn write_offset(n: u64) {
    if let Ok(bucket) = wasi::keyvalue::store::open(STATE_BUCKET) {
        let _ = bucket.set(OFFSET_KEY, n.to_string().as_bytes());
    }
}

fn http_get(host: &str, path_and_query: &str) -> Result<Vec<u8>, String> {
    let headers = Fields::new();
    headers
        .set(&"accept".to_string(), &[b"application/json".to_vec()])
        .map_err(|e| format!("set accept: {e:?}"))?;

    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Get)
        .map_err(|_| "set method".to_string())?;
    req.set_scheme(Some(&Scheme::Https))
        .map_err(|_| "set scheme".to_string())?;
    req.set_authority(Some(&host.to_string()))
        .map_err(|_| "set authority".to_string())?;
    req.set_path_with_query(Some(&path_and_query.to_string()))
        .map_err(|_| "set path".to_string())?;

    let outgoing_body = req.body().map_err(|_| "no outgoing body".to_string())?;
    OutgoingBody::finish(outgoing_body, None).map_err(|e| format!("finish: {e:?}"))?;

    let future_resp = outgoing_handler::handle(req, None).map_err(|e| format!("handle: {e:?}"))?;
    future_resp.subscribe().block();
    let resp = future_resp
        .get()
        .ok_or_else(|| "response missing".to_string())?
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
    Ok(buf)
}

#[derive(Deserialize)]
struct TgResponse {
    ok: bool,
    #[serde(default)]
    result: Vec<Value>,
    #[serde(default)]
    description: Option<String>,
}

fn publish(subject: &str, body: Vec<u8>) {
    let _ = wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
        subject: subject.to_string(),
        reply_to: None,
        body,
    });
}

fn max_update_id(updates: &[Value]) -> Option<u64> {
    updates
        .iter()
        .filter_map(|v| v.get("update_id")?.as_u64())
        .max()
}

fn poll_once(token: &str, offset: u64, timeout_s: u32) -> Result<Vec<Value>, String> {
    let path = format!("/bot{token}/getUpdates?offset={offset}&timeout={timeout_s}");
    let bytes = http_get("api.telegram.org", &path)?;
    let parsed: TgResponse = serde_json::from_slice(&bytes).map_err(|e| format!("decode: {e}"))?;
    if !parsed.ok {
        return Err(parsed
            .description
            .unwrap_or_else(|| "telegram returned ok=false".to_string()));
    }
    Ok(parsed.result)
}

fn tick_once() {
    // Always re-arm the loop first.
    publish(TICK_SUBJECT, Vec::new());

    let Some(token) = cfg("telegram.bot_token") else {
        log(
            wasi::logging::logging::Level::Warn,
            "telegram.bot_token not configured; idling",
        );
        sleep_ms(2_000);
        return;
    };
    let timeout_s: u32 = cfg("telegram.poll_timeout_s")
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);
    let idle_ms: u64 = cfg("telegram.poll_idle_ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    let offset = read_offset();
    match poll_once(&token, offset, timeout_s) {
        Ok(updates) => {
            for u in &updates {
                let bytes = serde_json::to_vec(u).unwrap_or_default();
                publish(RAW_SUBJECT, bytes);
            }
            if let Some(m) = max_update_id(&updates) {
                write_offset(m + 1);
            }
            if updates.is_empty() {
                sleep_ms(idle_ms);
            }
        }
        Err(e) => {
            log(
                wasi::logging::logging::Level::Warn,
                &format!("poll error: {e}; backoff 2s"),
            );
            sleep_ms(2_000);
        }
    }
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        if msg.subject != TICK_SUBJECT {
            return Ok(());
        }
        tick_once();
        Ok(())
    }
}

export!(Component);
