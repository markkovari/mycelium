// CLI ↔ Telegram pairing state machine.
//
// Exports mycelium:pairing/pairing.
// Also exposes request-reply over NATS so mycelium-cli (native binary) can call it:
//   mycelium.pair.request         → request-code
//   mycelium.pair.complete        → complete
//   mycelium.pair.get-by-session  → get-by-session
//   mycelium.pair.get-by-chat     → get-by-chat
//   mycelium.pair.unpair          → unpair
//
// KV bucket: mycelium-channel-sessions
//   pair/code/{CODE}         → PendingPair JSON
//   pair/session/{session}   → PairInfo JSON
//   pair/chat/{chat_id}      → PairInfo JSON
wit_bindgen::generate!({
    path: "wit",
    world: "session-bridge",
    generate_all,
});

use serde::{Deserialize, Serialize};

const BUCKET: &str = "mycelium-channel-sessions";
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

use exports::mycelium::pairing::pairing::{PairCode, PairInfo, PairRequest};
use mycelium::types::types::DomainError;

#[derive(Serialize, Deserialize)]
struct PendingPair {
    code: String,
    session_id: String,
    agent_id: String,
    expires_at: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct PairInfoJson {
    session_id: String,
    chat_id: String,
    conversation_id: String,
    paired_at: u64,
}

impl From<&PairInfoJson> for PairInfo {
    fn from(p: &PairInfoJson) -> Self {
        Self {
            session_id: p.session_id.clone(),
            chat_id: p.chat_id.clone(),
            conversation_id: p.conversation_id.clone(),
            paired_at: p.paired_at,
        }
    }
}

fn now_secs() -> u64 {
    wasi::clocks::wall_clock::now().seconds
}

fn open() -> Result<wasi::keyvalue::store::Bucket, DomainError> {
    wasi::keyvalue::store::open(BUCKET).map_err(|e| DomainError::Backend(format!("{e:?}")))
}

fn gen_code() -> String {
    let r = wasi::random::random::get_random_bytes(5);
    r.iter()
        .map(|b| CODE_ALPHABET[(*b as usize) % CODE_ALPHABET.len()] as char)
        .collect()
}

fn read_json<T: serde::de::DeserializeOwned>(
    bucket: &wasi::keyvalue::store::Bucket,
    key: &str,
) -> Result<Option<T>, DomainError> {
    match bucket
        .get(key)
        .map_err(|e| DomainError::Backend(format!("{e:?}")))?
    {
        Some(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).map_err(|e| DomainError::Internal(e.to_string()))?,
        )),
        None => Ok(None),
    }
}

fn write_json<T: Serialize>(
    bucket: &wasi::keyvalue::store::Bucket,
    key: &str,
    val: &T,
) -> Result<(), DomainError> {
    let json = serde_json::to_vec(val).map_err(|e| DomainError::Internal(e.to_string()))?;
    bucket
        .set(key, &json)
        .map_err(|e| DomainError::Backend(format!("{e:?}")))
}

struct Component;

impl exports::mycelium::pairing::pairing::Guest for Component {
    fn request_code(req: PairRequest) -> Result<PairCode, DomainError> {
        let bucket = open()?;
        let ttl = req.expires_in.min(300).max(30) as u64;
        let expires_at = now_secs() + ttl;
        // Try up to 8 times to find a free code.
        for _ in 0..8 {
            let code = gen_code();
            let key = format!("pair/code/{code}");
            if read_json::<PendingPair>(&bucket, &key)?.is_none() {
                let pending = PendingPair {
                    code: code.clone(),
                    session_id: req.session_id.clone(),
                    agent_id: req.agent_id.clone(),
                    expires_at,
                };
                write_json(&bucket, &key, &pending)?;
                return Ok(PairCode { code, expires_at });
            }
        }
        Err(DomainError::Internal("could not allocate code".into()))
    }

    fn complete(code: String, chat_id: String) -> Result<PairInfo, DomainError> {
        let bucket = open()?;
        let code_key = format!("pair/code/{code}");
        let pending: PendingPair = read_json(&bucket, &code_key)?
            .ok_or_else(|| DomainError::NotFound(format!("code {code}")))?;
        if pending.expires_at < now_secs() {
            let _ = bucket.delete(&code_key);
            return Err(DomainError::Validation("code expired".into()));
        }
        // Create a conversation via mycelium:conversation/conversations
        let conv = mycelium::conversation::conversations::create(
            &pending.agent_id,
            Some(&format!("CLI:{}↔chat:{chat_id}", pending.session_id)),
        )
        .map_err(|e| DomainError::Backend(format!("conv create: {e:?}")))?;

        let info = PairInfoJson {
            session_id: pending.session_id.clone(),
            chat_id: chat_id.clone(),
            conversation_id: conv.id,
            paired_at: now_secs(),
        };
        write_json(&bucket, &format!("pair/session/{}", info.session_id), &info)?;
        write_json(&bucket, &format!("pair/chat/{chat_id}"), &info)?;
        let _ = bucket.delete(&code_key);
        Ok(PairInfo::from(&info))
    }

    fn get_by_session(session_id: String) -> Result<Option<PairInfo>, DomainError> {
        let bucket = open()?;
        Ok(
            read_json::<PairInfoJson>(&bucket, &format!("pair/session/{session_id}"))?
                .as_ref()
                .map(PairInfo::from),
        )
    }

    fn get_by_chat(chat_id: String) -> Result<Option<PairInfo>, DomainError> {
        let bucket = open()?;
        Ok(
            read_json::<PairInfoJson>(&bucket, &format!("pair/chat/{chat_id}"))?
                .as_ref()
                .map(PairInfo::from),
        )
    }

    fn unpair(session_id: String) -> Result<(), DomainError> {
        let bucket = open()?;
        let session_key = format!("pair/session/{session_id}");
        if let Some(info) = read_json::<PairInfoJson>(&bucket, &session_key)? {
            let _ = bucket.delete(&format!("pair/chat/{}", info.chat_id));
        }
        let _ = bucket.delete(&session_key);
        Ok(())
    }
}

// NATS RPC façade for mycelium-cli (native binary, no WIT linkage).

fn reply(reply_to: Option<String>, body: Vec<u8>) {
    if let Some(subj) = reply_to {
        let _ =
            wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
                subject: subj,
                reply_to: None,
                body,
            });
    }
}

#[derive(Serialize)]
struct ErrResp {
    error: String,
}

fn err_resp(msg: String) -> Vec<u8> {
    serde_json::to_vec(&ErrResp { error: msg }).unwrap()
}

#[derive(Deserialize)]
struct PairReqJson {
    session_id: String,
    agent_id: String,
    expires_in: u32,
}

#[derive(Deserialize)]
struct CompleteReq {
    code: String,
    chat_id: String,
}
#[derive(Deserialize)]
struct SessionReq {
    session_id: String,
}
#[derive(Deserialize)]
struct ChatReq {
    chat_id: String,
}

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let body = msg.body;
        let reply_to = msg.reply_to;
        match msg.subject.as_str() {
            "mycelium.pair.request" => match serde_json::from_slice::<PairReqJson>(&body) {
                Ok(j) => {
                    let req = PairRequest {
                        session_id: j.session_id,
                        agent_id: j.agent_id,
                        expires_in: j.expires_in,
                    };
                    match Self::request_code(req) {
                            Ok(code) => reply(reply_to, serde_json::to_vec(&serde_json::json!({"code": code.code, "expires_at": code.expires_at})).unwrap()),
                            Err(e) => reply(reply_to, err_resp(format!("{e:?}"))),
                        }
                }
                Err(e) => reply(reply_to, err_resp(e.to_string())),
            },
            "mycelium.pair.complete" => match serde_json::from_slice::<CompleteReq>(&body) {
                Ok(req) => match Self::complete(req.code, req.chat_id) {
                    Ok(info) => reply(
                        reply_to,
                        serde_json::to_vec(&serde_json::json!({
                            "session_id": info.session_id,
                            "chat_id": info.chat_id,
                            "conversation_id": info.conversation_id,
                            "paired_at": info.paired_at,
                        }))
                        .unwrap(),
                    ),
                    Err(e) => reply(reply_to, err_resp(format!("{e:?}"))),
                },
                Err(e) => reply(reply_to, err_resp(e.to_string())),
            },
            "mycelium.pair.get-by-session" => match serde_json::from_slice::<SessionReq>(&body) {
                Ok(req) => match Self::get_by_session(req.session_id) {
                    Ok(info) => reply(
                        reply_to,
                        serde_json::to_vec(&info.as_ref().map(|i| {
                            serde_json::json!({
                                "session_id": i.session_id,
                                "chat_id": i.chat_id,
                                "conversation_id": i.conversation_id,
                                "paired_at": i.paired_at,
                            })
                        }))
                        .unwrap(),
                    ),
                    Err(e) => reply(reply_to, err_resp(format!("{e:?}"))),
                },
                Err(e) => reply(reply_to, err_resp(e.to_string())),
            },
            "mycelium.pair.get-by-chat" => match serde_json::from_slice::<ChatReq>(&body) {
                Ok(req) => match Self::get_by_chat(req.chat_id) {
                    Ok(info) => reply(
                        reply_to,
                        serde_json::to_vec(&info.as_ref().map(|i| {
                            serde_json::json!({
                                "session_id": i.session_id,
                                "chat_id": i.chat_id,
                                "conversation_id": i.conversation_id,
                                "paired_at": i.paired_at,
                            })
                        }))
                        .unwrap(),
                    ),
                    Err(e) => reply(reply_to, err_resp(format!("{e:?}"))),
                },
                Err(e) => reply(reply_to, err_resp(e.to_string())),
            },
            "mycelium.pair.unpair" => match serde_json::from_slice::<SessionReq>(&body) {
                Ok(req) => match Self::unpair(req.session_id) {
                    Ok(()) => reply(
                        reply_to,
                        serde_json::to_vec(&serde_json::json!({"ok": true})).unwrap(),
                    ),
                    Err(e) => reply(reply_to, err_resp(format!("{e:?}"))),
                },
                Err(e) => reply(reply_to, err_resp(e.to_string())),
            },
            _ => {}
        }
        Ok(())
    }
}

// Helper alias for the static-method calls above.
impl Component {
    fn request_code(req: PairRequest) -> Result<PairCode, DomainError> {
        <Component as exports::mycelium::pairing::pairing::Guest>::request_code(req)
    }
    fn complete(code: String, chat_id: String) -> Result<PairInfo, DomainError> {
        <Component as exports::mycelium::pairing::pairing::Guest>::complete(code, chat_id)
    }
    fn get_by_session(session_id: String) -> Result<Option<PairInfo>, DomainError> {
        <Component as exports::mycelium::pairing::pairing::Guest>::get_by_session(session_id)
    }
    fn get_by_chat(chat_id: String) -> Result<Option<PairInfo>, DomainError> {
        <Component as exports::mycelium::pairing::pairing::Guest>::get_by_chat(chat_id)
    }
    fn unpair(session_id: String) -> Result<(), DomainError> {
        <Component as exports::mycelium::pairing::pairing::Guest>::unpair(session_id)
    }
}

export!(Component);
