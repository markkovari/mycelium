// Channel router: normalises per-channel raw events to mycelium.channel.in
// and demuxes /admin commands.
//
// Currently handles the Telegram raw stream. Add per-channel handler blocks
// (Slack, CLI, web) by extending `handle_telegram_raw` / new subject matches.
wit_bindgen::generate!({
    path: "wit",
    world: "channel-router",
    generate_all,
});

use serde::Serialize;
use serde_json::Value;

const TELEGRAM_RAW: &str = "mycelium.channel.telegram.raw";
const CHANNEL_IN: &str = "mycelium.channel.in";

fn log(level: wasi::logging::logging::Level, msg: &str) {
    wasi::logging::logging::log(level, "channel-router", msg);
}

fn publish(subject: &str, body: Vec<u8>) {
    let _ = wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
        subject: subject.to_string(),
        reply_to: None,
        body,
    });
}

#[derive(Serialize)]
struct CanonicalMessage<'a> {
    channel: &'a str,
    channel_msg_id: String,
    sender_id: String,
    text: &'a str,
    raw_json: &'a str,
}

fn try_agent_create(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("/agent create ") else {
        return false;
    };
    let mut parts = rest.splitn(3, ' ');
    let id = match parts.next() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return false,
    };
    let model = match parts.next() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return false,
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
    match mycelium::agent::agent_registry::create(&config) {
        Ok(_) => true,
        Err(e) => {
            log(
                wasi::logging::logging::Level::Warn,
                &format!("agent_registry::create failed: {e:?}"),
            );
            true
        }
    }
}

fn handle_telegram_raw(body: &[u8]) {
    let update: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            log(
                wasi::logging::logging::Level::Warn,
                &format!("invalid raw update json: {e}"),
            );
            return;
        }
    };
    let Some(message) = update.get("message") else {
        return;
    };
    let chat_id = message
        .pointer("/chat/id")
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())
        .unwrap_or_default();
    let message_id = message
        .get("message_id")
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())
        .unwrap_or_default();
    let text = message.get("text").and_then(|v| v.as_str()).unwrap_or("");

    // Admin commands short-circuit canonical publish.
    if try_agent_create(text) {
        return;
    }

    let raw_json = update.to_string();
    let canonical = CanonicalMessage {
        channel: "telegram",
        channel_msg_id: message_id,
        sender_id: chat_id,
        text,
        raw_json: &raw_json,
    };
    if let Ok(bytes) = serde_json::to_vec(&canonical) {
        publish(CHANNEL_IN, bytes);
    }
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        match msg.subject.as_str() {
            TELEGRAM_RAW => handle_telegram_raw(&msg.body),
            _ => {}
        }
        Ok(())
    }
}

export!(Component);
