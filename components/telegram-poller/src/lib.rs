// Telegram transport adapter — long polling.
//
// Drains the Telegram Bot API and emits raw updates on
// `mycelium.channel.telegram.raw`. Self-ticks on
// `mycelium.telegram.poll.tick`. Persists update_id offset in
// `mycelium-telegram-poller-state` KV.
//
// All Telegram I/O lives in the `mycelium-telegram` crate so this file
// stays orchestration-only.
//
// Config (wasi:config/store):
//   telegram.bot_token        BotFather token (required)
//   telegram.poll_timeout_s   long-poll timeout (default 25)
//   telegram.poll_idle_ms     sleep between empty polls (default 500)
wit_bindgen::generate!({
    path: "wit",
    world: "telegram-poller",
    generate_all,
});

use mycelium_telegram::TgClient;

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

fn publish(subject: &str, body: Vec<u8>) {
    let _ = wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
        subject: subject.to_string(),
        reply_to: None,
        body,
    });
}

fn tick_once() {
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

    let client = TgClient::new(token);
    let offset = read_offset();

    let ack_emoji = cfg("telegram.ack_emoji").unwrap_or_else(|| "\u{1F440}".to_string()); // 👀
    let ack_enabled = cfg("telegram.ack_emoji_disabled")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
        == false;

    match client.get_updates(offset, timeout_s) {
        Ok(updates) => {
            for u in &updates {
                let bytes = serde_json::to_vec(u).unwrap_or_default();
                publish(RAW_SUBJECT, bytes);
                if ack_enabled {
                    if let Some(msg) = u.message.as_ref() {
                        if let Err(e) = client.set_reaction(msg.chat.id, msg.message_id, &ack_emoji) {
                            log(
                                wasi::logging::logging::Level::Debug,
                                &format!("set_reaction failed: {e}"),
                            );
                        }
                    }
                }
            }
            if let Some(max) = updates.iter().map(|u| u.update_id).max() {
                write_offset(max + 1);
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
