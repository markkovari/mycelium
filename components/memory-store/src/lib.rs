// Agent memory (KV-backed).
// Exports mycelium:memory/memory — called directly by executor and agent components.
// Also subscribes to mycelium.memory.> for request-reply access from native clients.
// KV bucket: mycelium-memory  key format: memory/{agent_id}/{key}
wit_bindgen::generate!({
    path: "wit",
    world: "memory-store",
    generate_all,
});

use serde::{Deserialize, Serialize};

const BUCKET: &str = "mycelium-memory";

fn k(agent_id: &str, key: &str) -> String {
    format!("memory/{agent_id}/{key}")
}

fn kv_err(e: wasi::keyvalue::store::Error) -> mycelium::types::types::DomainError {
    mycelium::types::types::DomainError::Backend(format!("kv: {e:?}"))
}

fn open() -> Result<wasi::keyvalue::store::Bucket, mycelium::types::types::DomainError> {
    wasi::keyvalue::store::open(BUCKET).map_err(kv_err)
}

struct Component;

impl exports::mycelium::memory::memory::Guest for Component {
    fn set(
        agent_id: String,
        key: String,
        value: String,
        _ttl_secs: Option<u64>,
    ) -> Result<(), mycelium::types::types::DomainError> {
        let bucket = open()?;
        bucket
            .set(&k(&agent_id, &key), value.as_bytes())
            .map_err(kv_err)
    }

    fn get(
        agent_id: String,
        key: String,
    ) -> Result<Option<String>, mycelium::types::types::DomainError> {
        let bucket = open()?;
        match bucket.get(&k(&agent_id, &key)).map_err(kv_err)? {
            Some(bytes) => Ok(Some(String::from_utf8(bytes).map_err(|e| {
                mycelium::types::types::DomainError::Backend(e.to_string())
            })?)),
            None => Ok(None),
        }
    }

    fn delete(agent_id: String, key: String) -> Result<(), mycelium::types::types::DomainError> {
        let bucket = open()?;
        bucket.delete(&k(&agent_id, &key)).map_err(kv_err)
    }

    fn list_keys(agent_id: String) -> Result<Vec<String>, mycelium::types::types::DomainError> {
        let bucket = open()?;
        let prefix = format!("memory/{agent_id}/");
        let resp = bucket.list_keys(None).map_err(kv_err)?;
        Ok(resp
            .keys
            .into_iter()
            .filter(|kk| kk.starts_with(&prefix))
            .map(|kk| kk.trim_start_matches(&prefix).to_string())
            .collect())
    }

    fn set_many(
        agent_id: String,
        entries: Vec<(String, String)>,
    ) -> Result<(), mycelium::types::types::DomainError> {
        let bucket = open()?;
        for (key, value) in entries {
            bucket
                .set(&k(&agent_id, &key), value.as_bytes())
                .map_err(kv_err)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct MemoryGetReq {
    agent_id: String,
    key: String,
}
#[derive(Serialize)]
struct MemoryGetResp {
    value: Option<String>,
}
#[derive(Deserialize)]
struct MemorySetReq {
    agent_id: String,
    key: String,
    value: String,
    ttl_secs: Option<u64>,
}
#[derive(Deserialize)]
struct MemoryDelReq {
    agent_id: String,
    key: String,
}
#[derive(Deserialize)]
struct MemoryListReq {
    agent_id: String,
}
#[derive(Serialize)]
struct MemoryListResp {
    keys: Vec<String>,
}
#[derive(Serialize)]
struct MemoryAck {
    ok: bool,
    error: Option<String>,
}

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

fn ack(reply_to: Option<String>, r: Result<(), mycelium::types::types::DomainError>) {
    let resp = match r {
        Ok(()) => MemoryAck {
            ok: true,
            error: None,
        },
        Err(e) => MemoryAck {
            ok: false,
            error: Some(format!("{e:?}")),
        },
    };
    reply(reply_to, serde_json::to_vec(&resp).unwrap());
}

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let body = msg.body;
        match msg.subject.as_str() {
            "mycelium.memory.get" => {
                let req: MemoryGetReq = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
                let value = <Component as exports::mycelium::memory::memory::Guest>::get(
                    req.agent_id,
                    req.key,
                )
                .map_err(|e| format!("{e:?}"))?;
                reply(
                    msg.reply_to,
                    serde_json::to_vec(&MemoryGetResp { value }).unwrap(),
                );
            }
            "mycelium.memory.set" => {
                let req: MemorySetReq = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
                let r = <Component as exports::mycelium::memory::memory::Guest>::set(
                    req.agent_id,
                    req.key,
                    req.value,
                    req.ttl_secs,
                );
                ack(msg.reply_to, r);
            }
            "mycelium.memory.delete" => {
                let req: MemoryDelReq = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
                let r = <Component as exports::mycelium::memory::memory::Guest>::delete(
                    req.agent_id,
                    req.key,
                );
                ack(msg.reply_to, r);
            }
            "mycelium.memory.list" => {
                let req: MemoryListReq =
                    serde_json::from_slice(&body).map_err(|e| e.to_string())?;
                let keys = <Component as exports::mycelium::memory::memory::Guest>::list_keys(
                    req.agent_id,
                )
                .map_err(|e| format!("{e:?}"))?;
                reply(
                    msg.reply_to,
                    serde_json::to_vec(&MemoryListResp { keys }).unwrap(),
                );
            }
            _ => {}
        }
        Ok(())
    }
}

export!(Component);
