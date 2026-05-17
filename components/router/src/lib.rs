// Event fan-out router.
// Subscribes to mycelium.event.> and fans messages out to registered target subjects.
// Rules stored in mycelium-router-rules KV bucket.
//
// Rule key format: rule/{subject_pattern}
// Rule value (JSON): {subject_pattern, targets: [String], filter_jmespath: Option<String>}
//
// Rules are seeded externally (via mycelium-cli or `nats kv put`).
// The component reads rules on every incoming message.
wit_bindgen::generate!({
    path: "wit",
    world: "router",
    generate_all,
});

use serde::{Deserialize, Serialize};

const BUCKET: &str = "mycelium-router-rules";

#[derive(Serialize, Deserialize, Clone)]
struct RuleJson {
    subject_pattern: String,
    targets: Vec<String>,
    filter_jmespath: Option<String>,
}

fn open() -> Option<wasi::keyvalue::store::Bucket> {
    wasi::keyvalue::store::open(BUCKET).ok()
}

/// NATS subject pattern matcher.
/// `*` matches one segment; `>` matches one or more segments at the tail.
fn matches(pattern: &str, subject: &str) -> bool {
    let p: Vec<&str> = pattern.split('.').collect();
    let s: Vec<&str> = subject.split('.').collect();
    let mut i = 0;
    while i < p.len() {
        if p[i] == ">" {
            return i < s.len();
        }
        if i >= s.len() {
            return false;
        }
        if p[i] != "*" && p[i] != s[i] {
            return false;
        }
        i += 1;
    }
    i == s.len()
}

fn load_rules() -> Vec<RuleJson> {
    let bucket = match open() {
        Some(b) => b,
        None => return Vec::new(),
    };
    let resp = match bucket.list_keys(None) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for key in resp.keys.iter().filter(|k| k.starts_with("rule/")) {
        if let Ok(Some(bytes)) = bucket.get(key) {
            if let Ok(r) = serde_json::from_slice::<RuleJson>(&bytes) {
                out.push(r);
            }
        }
    }
    out
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let subject = msg.subject.clone();
        let body = msg.body.clone();
        for rule in load_rules() {
            if matches(&rule.subject_pattern, &subject) {
                for target in &rule.targets {
                    let out = wasmcloud::messaging::types::BrokerMessage {
                        subject: target.clone(),
                        reply_to: None,
                        body: body.clone(),
                    };
                    let _ = wasmcloud::messaging::consumer::publish(&out);
                }
            }
        }
        Ok(())
    }
}

export!(Component);
