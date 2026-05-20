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

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const TELEGRAM_RAW: &str = "mycelium.channel.telegram.raw";
const CHANNEL_IN: &str = "mycelium.channel.in";
const STEP_RESULT: &str = "mycelium.step.result";
const TASK_SUBMIT: &str = "mycelium.task.submit";
const PENDING_BUCKET: &str = "mycelium-channel-pending";

fn cfg(key: &str) -> Option<String> {
    wasi::config::store::get(key).ok().flatten()
}

fn random_uuid_v4() -> String {
    // RFC 4122 v4: 16 random bytes, set version + variant.
    let bytes: [u8; 16] = wasi::random::random::get_random_bytes(16)
        .as_slice()
        .try_into()
        .unwrap_or([0; 16]);
    let mut b = bytes;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

fn seen_update(update_id: u64) -> bool {
    // wash 2.1.0 fans every NATS message to multiple component instances in
    // parallel, even with pool_size=1 and max_invocations=1. wasi:keyvalue
    // atomics crash the NATS plugin, so we marker-CAS instead: write a random
    // claim token to the seen key, then read it back. If it matches we won; if
    // it differs, another parallel instance overwrote us and we bail.
    let Ok(bucket) = wasi::keyvalue::store::open("mycelium-channel-pending") else {
        return false;
    };
    let key = format!("seen/{update_id}");
    if matches!(bucket.exists(&key), Ok(true)) {
        // Definitively seen earlier; might be our own marker or an older "1".
        // Either way treat as duplicate.
        let was_our_marker = match bucket.get(&key) {
            Ok(Some(b)) => b.starts_with(b"claim-"),
            _ => false,
        };
        // If it's an older "1" marker (pre-marker code), still a dup.
        let _ = was_our_marker;
        return true;
    }
    let marker = format!("claim-{}", random_uuid_v4());
    if bucket.set(&key, marker.as_bytes()).is_err() {
        return true;
    }
    match bucket.get(&key) {
        Ok(Some(stored)) if stored == marker.as_bytes() => false,
        _ => true,
    }
}

fn pick_default_agent_for_chat(chat_id: &str) -> Option<String> {
    if let Some(id) = load_chat_agent(chat_id) {
        if !id.is_empty() {
            return Some(id);
        }
    }
    if let Some(id) = cfg("default.agent_id") {
        if !id.is_empty() && id != "auto" {
            return Some(id);
        }
    }
    mycelium::agent::agent_registry::list_agents()
        .ok()
        .and_then(|v| v.into_iter().next().map(|a| a.id))
}

#[derive(Serialize, Deserialize)]
struct PendingTask {
    channel: String,
    chat_id: String,
    #[serde(default)]
    conversation_id: String,
}

fn load_pending(task_id: &str) -> Option<PendingTask> {
    let bucket = wasi::keyvalue::store::open(PENDING_BUCKET).ok()?;
    let bytes = bucket.get(task_id).ok().flatten()?;
    serde_json::from_slice(&bytes).ok()
}

fn delete_pending(task_id: &str) {
    if let Ok(bucket) = wasi::keyvalue::store::open(PENDING_BUCKET) {
        let _ = bucket.delete(task_id);
    }
}

fn now_iso() -> String {
    let now = wasi::clocks::wall_clock::now();
    let secs = now.seconds as i64;
    // RFC3339 UTC, fixed offset
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem / 60) % 60, rem % 60);
    // Simple Y/M/D from epoch — good enough for logging; agent doesn't parse it.
    let year_full = 1970 + (days / 365);
    let mo = ((days % 365) / 30) + 1;
    let d = ((days % 365) % 30) + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year_full, mo, d, h, m, s
    )
}

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

fn try_slash_command(chat_id: &str, text: &str) -> bool {
    if !text.starts_with('/') {
        return false;
    }
    let token = match cfg("telegram.bot_token") {
        Some(t) => t,
        None => return false,
    };
    let send = |msg: &str| {
        let _ = telegram_send(&token, chat_id, msg);
    };

    let mut it = text.splitn(2, ' ');
    let cmd = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();

    match cmd {
        "/help" | "/start" => {
            send("commands:\n/help — this menu\n/agents — list agents\n/agent set <id> — switch active agent for this chat\n/agent create <id> <model> <prompt> — register a new agent\n/reset — clear conversation history\n/memory — show last 10 messages\n/tools — list registered tools");
            true
        }
        "/agents" => {
            let active = load_chat_agent(chat_id).unwrap_or_else(|| "(none)".to_string());
            let list: String = match mycelium::agent::agent_registry::list_agents() {
                Ok(v) => {
                    if v.is_empty() {
                        "no agents yet — use /agent create <id> <model> <prompt>".to_string()
                    } else {
                        v.iter()
                            .map(|a| {
                                let marker = if a.id == active { "* " } else { "  " };
                                format!("{marker}{} ({})", a.id, a.model)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                }
                Err(e) => format!("list_agents failed: {e:?}"),
            };
            send(&list);
            true
        }
        "/agent" => {
            let mut sub = rest.splitn(2, ' ');
            match sub.next().unwrap_or("") {
                "set" => {
                    let id = sub.next().unwrap_or("").trim();
                    if id.is_empty() {
                        send("usage: /agent set <id>");
                    } else {
                        save_chat_agent(chat_id, id);
                        send(&format!("default agent for this chat → {id}"));
                    }
                    true
                }
                "create" => false, // existing handler picks it up
                _ => {
                    send("usage: /agent set <id> | /agent create <id> <model> <prompt>");
                    true
                }
            }
        }
        "/reset" => {
            if let Some(conv_id) = load_chat_conv(chat_id) {
                let _ = mycelium::conversation::conversations::delete(&conv_id);
                clear_chat_conv(chat_id);
                send("conversation cleared.");
            } else {
                send("nothing to reset.");
            }
            true
        }
        "/memory" => {
            let body = match load_chat_conv(chat_id) {
                None => "no conversation yet.".to_string(),
                Some(cid) => match mycelium::conversation::conversations::get_messages(&cid) {
                    Err(e) => format!("get_messages failed: {e:?}"),
                    Ok(msgs) => {
                        let tail: Vec<_> = msgs.iter().rev().take(10).collect();
                        if tail.is_empty() {
                            "(empty)".to_string()
                        } else {
                            tail.iter()
                                .rev()
                                .map(|m| {
                                    let role = match m.role {
                                        mycelium::types::types::MessageRole::User => "you",
                                        mycelium::types::types::MessageRole::Assistant => "bot",
                                        mycelium::types::types::MessageRole::System => "sys",
                                        mycelium::types::types::MessageRole::Tool => "tool",
                                    };
                                    let snippet: String =
                                        m.content.chars().take(120).collect();
                                    format!("{role}: {snippet}")
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                },
            };
            send(&body);
            true
        }
        "/tools" => {
            send(&list_registered_tools());
            true
        }
        "/quota" | "/budget" => {
            send(&format_quota_for_chat());
            true
        }
        _ => false,
    }
}

fn format_quota_for_chat() -> String {
    // The agent component writes its rate counters into mycelium-task-state
    // under keys `rate/<agent>/{min,day}/<bucket>`. We don't know which agent
    // is bound to this chat from here (cheap path), so we sum across all
    // agents for the current minute / day and report.
    let now = wasi::clocks::wall_clock::now();
    let minute = now.seconds / 60;
    let day = now.seconds / 86_400;
    let bucket = match wasi::keyvalue::store::open("mycelium-task-state") {
        Ok(b) => b,
        Err(_) => return "(no rate state)".to_string(),
    };
    let keys = bucket.list_keys(None).map(|r| r.keys).unwrap_or_default();
    let mut rpm_used = 0u64;
    let mut rpd_used = 0u64;
    for k in &keys {
        if k.ends_with(&format!("/min/{minute}")) || k.contains(&format!("/min/{minute}")) {
            if let Ok(Some(bytes)) = bucket.get(k) {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    rpm_used += s.parse::<u64>().unwrap_or(0);
                }
            }
        }
        if k.ends_with(&format!("/day/{day}")) || k.contains(&format!("/day/{day}")) {
            if let Ok(Some(bytes)) = bucket.get(k) {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    rpd_used += s.parse::<u64>().unwrap_or(0);
                }
            }
        }
    }
    let rpm_cap = cfg("llm.rpm").and_then(|s| s.parse::<u64>().ok()).unwrap_or(10);
    let rpd_cap = cfg("llm.rpd").and_then(|s| s.parse::<u64>().ok()).unwrap_or(200);
    format!(
        "quota:\n  RPM {rpm_used}/{rpm_cap}\n  RPD {rpd_used}/{rpd_cap}"
    )
}

fn list_registered_tools() -> String {
    let Ok(bucket) = wasi::keyvalue::store::open("mycelium-tools") else {
        return "tool registry unavailable.".to_string();
    };
    let keys = match bucket.list_keys(None) {
        Ok(r) => r.keys,
        Err(_) => return "tool registry list failed.".to_string(),
    };
    if keys.is_empty() {
        return "no tools registered.".to_string();
    }
    let mut out = String::from("registered tools:\n");
    for k in &keys {
        let desc = bucket
            .get(k)
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|v| v.get("description").and_then(|d| d.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();
        if desc.is_empty() {
            out.push_str(&format!("- {k}\n"));
        } else {
            out.push_str(&format!("- {k} — {desc}\n"));
        }
    }
    out
}

fn load_chat_agent(chat_id: &str) -> Option<String> {
    let bucket = wasi::keyvalue::store::open(PENDING_BUCKET).ok()?;
    let bytes = bucket.get(&format!("default-agent/{chat_id}")).ok().flatten()?;
    String::from_utf8(bytes).ok()
}

fn save_chat_agent(chat_id: &str, agent_id: &str) {
    if let Ok(bucket) = wasi::keyvalue::store::open(PENDING_BUCKET) {
        let _ = bucket.set(&format!("default-agent/{chat_id}"), agent_id.as_bytes());
    }
}

fn clear_chat_conv(chat_id: &str) {
    if let Ok(bucket) = wasi::keyvalue::store::open(PENDING_BUCKET) {
        let _ = bucket.delete(&format!("conv/{chat_id}"));
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
    // Dedupe by update_id. wash 2.1.0 may deliver the same raw subject to
    // multiple concurrent handler instances; we only want to fire one task
    // per Telegram update.
    let update_id = update.get("update_id").and_then(|v| v.as_u64());
    if let Some(uid) = update_id {
        if seen_update(uid) {
            return;
        }
    }
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

    // Admin commands short-circuit canonical publish + LLM dispatch.
    if try_slash_command(&chat_id, text) {
        return;
    }
    if try_agent_create(text) {
        return;
    }

    let raw_json = update.to_string();
    let canonical = CanonicalMessage {
        channel: "telegram",
        channel_msg_id: message_id.clone(),
        sender_id: chat_id.clone(),
        text,
        raw_json: &raw_json,
    };
    if let Ok(bytes) = serde_json::to_vec(&canonical) {
        publish(CHANNEL_IN, bytes);
    }

    if text.is_empty() {
        return;
    }
    let Some(agent_id) = pick_default_agent_for_chat(&chat_id) else {
        log(
            wasi::logging::logging::Level::Warn,
            "no agent registered; create one with /agent create or POST /agents",
        );
        return;
    };

    // Show typing indicator via direct Telegram API (wash messaging plugin
    // doesn't reliably route to telegram-out, so we call Telegram inline).
    if let Some(token) = cfg("telegram.bot_token") {
        let _ = telegram_chat_action(&token, &chat_id, "typing");
    }

    // Stable conv_id per chat — load if exists, otherwise create.
    let conv_id = match load_chat_conv(&chat_id) {
        Some(id) => id,
        None => {
            match mycelium::conversation::conversations::create(&agent_id, None) {
                Ok(c) => {
                    save_chat_conv(&chat_id, &c.id);
                    c.id
                }
                Err(e) => {
                    log(
                        wasi::logging::logging::Level::Warn,
                        &format!("conversations::create failed: {e:?}"),
                    );
                    return;
                }
            }
        }
    };

    // Append user message so the agent can read full history on the next step.
    if let Err(e) = mycelium::conversation::conversations::append_message(
        &conv_id,
        mycelium::types::types::MessageRole::User,
        text,
        None,
    ) {
        log(
            wasi::logging::logging::Level::Warn,
            &format!("append user message failed: {e:?}"),
        );
    }

    // Deterministic task_id from update_id so parallel router instances
    // produce identical task.submit messages — executor then dedups on task_id
    // and only the first instance does I/O (sends the placeholder, etc.).
    let task_id = match update_id {
        Some(uid) => format!("tg-{uid}"),
        None => random_uuid_v4(),
    };
    let task = json!({
        "id": task_id,
        "conversation_id": conv_id,
        "agent_id": agent_id,
        "input": text,
        "created_at": now_iso(),
    });
    save_pending_full(&task_id, "telegram", &chat_id, &conv_id);
    if let Ok(bytes) = serde_json::to_vec(&task) {
        publish(TASK_SUBMIT, bytes);
    }
}

fn load_chat_conv(chat_id: &str) -> Option<String> {
    let bucket = wasi::keyvalue::store::open(PENDING_BUCKET).ok()?;
    let bytes = bucket.get(&format!("conv/{chat_id}")).ok().flatten()?;
    String::from_utf8(bytes).ok()
}

fn save_chat_conv(chat_id: &str, conv_id: &str) {
    if let Ok(bucket) = wasi::keyvalue::store::open(PENDING_BUCKET) {
        let _ = bucket.set(&format!("conv/{chat_id}"), conv_id.as_bytes());
    }
}

fn save_pending_full(task_id: &str, channel: &str, chat_id: &str, conversation_id: &str) {
    let Ok(bucket) = wasi::keyvalue::store::open(PENDING_BUCKET) else { return };
    let pt = json!({
        "channel": channel,
        "chat_id": chat_id,
        "conversation_id": conversation_id,
    });
    if let Ok(bytes) = serde_json::to_vec(&pt) {
        let _ = bucket.set(task_id, &bytes);
    }
}

fn handle_step_result(body: &[u8]) {
    let res: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return,
    };
    let Some(task_id) = res.get("task_id").and_then(|v| v.as_str()) else { return };
    let Some(pending) = load_pending(task_id) else { return };
    let text = res
        .get("output")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            res.get("error")
                .and_then(|v| v.as_str())
                .map(|e| format!("(error: {e})"))
        })
        .unwrap_or_else(|| "(no response)".to_string());

    if pending.channel == "telegram" {
        // wash 2.1.0 messaging plugin does not deliver pubs to telegram-out's
        // wildcard subscription, so call Telegram directly here.
        if let Some(token) = cfg("telegram.bot_token") {
            if let Err(e) = telegram_send(&token, &pending.chat_id, &text) {
                log(
                    wasi::logging::logging::Level::Warn,
                    &format!("telegram send failed: {e}"),
                );
            }
        } else {
            log(
                wasi::logging::logging::Level::Warn,
                "telegram.bot_token not configured on channel-router",
            );
        }
    }
    delete_pending(task_id);
}

fn telegram_chat_action(token: &str, chat_id: &str, action: &str) -> Result<(), String> {
    telegram_post(token, "sendChatAction", &json!({"chat_id": chat_id, "action": action}))
}

fn telegram_send(token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    telegram_post(token, "sendMessage", &json!({"chat_id": chat_id, "text": text}))
}

fn telegram_post(token: &str, method: &str, body: &Value) -> Result<(), String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
    let body_bytes = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let headers = Fields::new();
    headers
        .set("content-type", &[b"application/json".to_vec()])
        .map_err(|e| format!("{e:?}"))?;
    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Post).map_err(|_| "method".to_string())?;
    req.set_scheme(Some(&Scheme::Https)).map_err(|_| "scheme".to_string())?;
    req.set_authority(Some("api.telegram.org"))
        .map_err(|_| "authority".to_string())?;
    req.set_path_with_query(Some(&format!("/bot{token}/{method}")))
        .map_err(|_| "path".to_string())?;
    let outgoing = req.body().map_err(|_| "body".to_string())?;
    {
        let stream = outgoing.write().map_err(|_| "stream".to_string())?;
        for chunk in body_bytes.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    OutgoingBody::finish(outgoing, None).map_err(|e| format!("{e:?}"))?;
    let fut = outgoing_handler::handle(req, None).map_err(|e| format!("{e:?}"))?;
    fut.subscribe().block();
    let _resp = fut
        .get()
        .ok_or("no resp")?
        .map_err(|_| "consumed".to_string())?
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        // step.result is consumed by executor (it sends the Telegram reply
        // inline since wash 2.1.0 won't deliver the same subject to two
        // distinct workloads reliably).
        if msg.subject == TELEGRAM_RAW {
            handle_telegram_raw(&msg.body);
        }
        Ok(())
    }
}

export!(Component);
