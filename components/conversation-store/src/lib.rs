// Conversation and message persistence.
// Exports mycelium:conversation/conversations.
// KV buckets: mycelium-conversations (metadata), mycelium-messages (per-conv message log).
// Key formats:
//   conv/{conversation_id}         → Conversation JSON
//   msg/{conversation_id}/{msg_id} → Message JSON
wit_bindgen::generate!({
    path: "wit",
    world: "conversation-store",
    generate_all,
});

use serde::{Deserialize, Serialize};

const CONV_BUCKET: &str = "mycelium-conversations";
const MSG_BUCKET: &str = "mycelium-messages";

use mycelium::types::types::{Conversation, DomainError, Message, MessageRole};

#[derive(Serialize, Deserialize)]
struct ConvJson {
    id: String,
    agent_id: String,
    title: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<&Conversation> for ConvJson {
    fn from(c: &Conversation) -> Self {
        Self {
            id: c.id.clone(),
            agent_id: c.agent_id.clone(),
            title: c.title.clone(),
            created_at: c.created_at.clone(),
            updated_at: c.updated_at.clone(),
        }
    }
}

impl From<ConvJson> for Conversation {
    fn from(j: ConvJson) -> Self {
        Self {
            id: j.id,
            agent_id: j.agent_id,
            title: j.title,
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct MsgJson {
    id: String,
    role: String,
    content: String,
    tool_use_id: Option<String>,
    created_at: String,
}

fn role_to_str(r: MessageRole) -> &'static str {
    match r {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

fn role_from_str(s: &str) -> MessageRole {
    match s {
        "system" => MessageRole::System,
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "tool" => MessageRole::Tool,
        _ => MessageRole::User,
    }
}

impl From<&Message> for MsgJson {
    fn from(m: &Message) -> Self {
        Self {
            id: m.id.clone(),
            role: role_to_str(m.role).to_string(),
            content: m.content.clone(),
            tool_use_id: m.tool_use_id.clone(),
            created_at: m.created_at.clone(),
        }
    }
}

impl From<MsgJson> for Message {
    fn from(j: MsgJson) -> Self {
        Self {
            id: j.id,
            role: role_from_str(&j.role),
            content: j.content,
            tool_use_id: j.tool_use_id,
            created_at: j.created_at,
        }
    }
}

fn kv_err(e: wasi::keyvalue::store::Error) -> DomainError {
    DomainError::Backend(format!("kv: {e:?}"))
}

fn open(bucket: &str) -> Result<wasi::keyvalue::store::Bucket, DomainError> {
    wasi::keyvalue::store::open(bucket).map_err(kv_err)
}

fn now_iso() -> String {
    let d = wasi::clocks::wall_clock::now();
    // Seconds since epoch + nanos; produce ISO-8601 UTC manually.
    let secs = d.seconds as i64;
    let (year, month, day, hh, mm, ss) = secs_to_ymdhms(secs);
    format!(
        "{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}.{:09}Z",
        d.nanoseconds
    )
}

// Minimal civil-from-days conversion (proleptic Gregorian) — RFC 7232 / Howard Hinnant.
fn secs_to_ymdhms(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let hh = rem / 3600;
    let mm = (rem / 60) % 60;
    let ss = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if m <= 2 { y + 1 } else { y };
    (yr, m, d, hh, mm, ss)
}

fn random_uuid_v7() -> String {
    // 16-byte UUID v7: 48-bit unix ms timestamp + 4-bit version (7) + 12-bit rand + 2-bit variant + 62-bit rand
    let now = wasi::clocks::wall_clock::now();
    let ms: u64 = now.seconds * 1_000 + (now.nanoseconds as u64) / 1_000_000;
    let r1 = wasi::random::random::get_random_u64();
    let r2 = wasi::random::random::get_random_u64();
    let mut b = [0u8; 16];
    b[0..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
    b[6] = 0x70 | ((r1 >> 8) as u8 & 0x0f);
    b[7] = r1 as u8;
    b[8] = 0x80 | ((r2 >> 56) as u8 & 0x3f);
    b[9..16].copy_from_slice(&r2.to_be_bytes()[1..8]);
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

struct Component;

impl exports::mycelium::conversation::conversations::Guest for Component {
    fn create(agent_id: String, title: Option<String>) -> Result<Conversation, DomainError> {
        let id = random_uuid_v7();
        let now = now_iso();
        let conv = Conversation {
            id: id.clone(),
            agent_id,
            title,
            created_at: now.clone(),
            updated_at: now,
        };
        let bucket = open(CONV_BUCKET)?;
        let json = serde_json::to_vec(&ConvJson::from(&conv))
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        bucket.set(&format!("conv/{id}"), &json).map_err(kv_err)?;
        Ok(conv)
    }

    fn get(id: String) -> Result<Conversation, DomainError> {
        let bucket = open(CONV_BUCKET)?;
        match bucket.get(&format!("conv/{id}")).map_err(kv_err)? {
            Some(bytes) => {
                let j: ConvJson = serde_json::from_slice(&bytes)
                    .map_err(|e| DomainError::Internal(e.to_string()))?;
                Ok(j.into())
            }
            None => Err(DomainError::NotFound(id)),
        }
    }

    fn list_conversations(agent_id: String) -> Result<Vec<Conversation>, DomainError> {
        let bucket = open(CONV_BUCKET)?;
        let resp = bucket.list_keys(None).map_err(kv_err)?;
        let mut out = Vec::new();
        for key in resp.keys.iter().filter(|k| k.starts_with("conv/")) {
            if let Ok(Some(bytes)) = bucket.get(key) {
                if let Ok(j) = serde_json::from_slice::<ConvJson>(&bytes) {
                    if j.agent_id == agent_id {
                        out.push(j.into());
                    }
                }
            }
        }
        Ok(out)
    }

    fn delete(id: String) -> Result<(), DomainError> {
        let bucket = open(CONV_BUCKET)?;
        bucket.delete(&format!("conv/{id}")).map_err(kv_err)?;
        // Also delete messages
        let msgs = open(MSG_BUCKET)?;
        let prefix = format!("msg/{id}/");
        if let Ok(resp) = msgs.list_keys(None) {
            for key in resp.keys.iter().filter(|k| k.starts_with(&prefix)) {
                let _ = msgs.delete(key);
            }
        }
        Ok(())
    }

    fn append_message(
        conversation_id: String,
        role: MessageRole,
        content: String,
        tool_use_id: Option<String>,
    ) -> Result<Message, DomainError> {
        let msg_id = random_uuid_v7();
        let now = now_iso();
        let msg = Message {
            id: msg_id.clone(),
            role,
            content,
            tool_use_id,
            created_at: now.clone(),
        };
        let bucket = open(MSG_BUCKET)?;
        let json = serde_json::to_vec(&MsgJson::from(&msg))
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        // Prefix key with ms timestamp for stable ordering.
        let ms_now = wasi::clocks::wall_clock::now();
        let stamp = ms_now.seconds * 1_000_000_000 + ms_now.nanoseconds as u64;
        bucket
            .set(
                &format!("msg/{conversation_id}/{stamp:020}_{msg_id}"),
                &json,
            )
            .map_err(kv_err)?;
        // Touch conversation updated_at
        let conv_bucket = open(CONV_BUCKET)?;
        if let Ok(Some(bytes)) = conv_bucket.get(&format!("conv/{conversation_id}")) {
            if let Ok(mut j) = serde_json::from_slice::<ConvJson>(&bytes) {
                j.updated_at = now;
                if let Ok(updated) = serde_json::to_vec(&j) {
                    let _ = conv_bucket.set(&format!("conv/{conversation_id}"), &updated);
                }
            }
        }
        Ok(msg)
    }

    fn get_messages(conversation_id: String) -> Result<Vec<Message>, DomainError> {
        let bucket = open(MSG_BUCKET)?;
        let prefix = format!("msg/{conversation_id}/");
        let resp = bucket.list_keys(None).map_err(kv_err)?;
        let mut keys: Vec<_> = resp
            .keys
            .into_iter()
            .filter(|k| k.starts_with(&prefix))
            .collect();
        keys.sort();
        let mut out = Vec::new();
        for key in keys {
            if let Ok(Some(bytes)) = bucket.get(&key) {
                if let Ok(j) = serde_json::from_slice::<MsgJson>(&bytes) {
                    out.push(j.into());
                }
            }
        }
        Ok(out)
    }

    fn get_messages_after(
        conversation_id: String,
        after_id: String,
    ) -> Result<Vec<Message>, DomainError> {
        let all =
            <Component as exports::mycelium::conversation::conversations::Guest>::get_messages(
                conversation_id,
            )?;
        let mut found = false;
        let mut out = Vec::new();
        for m in all {
            if found {
                out.push(m);
            } else if m.id == after_id {
                found = true;
            }
        }
        Ok(out)
    }
}

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(_msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        // No-op for now — clients invoke conversation-store via WIT exports.
        // Optional RPC façade can be added later if cli needs it.
        Ok(())
    }
}

export!(Component);
