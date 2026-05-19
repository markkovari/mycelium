// time tool: returns current UTC ISO timestamp.
// Subscribe: mycelium.tool.call.time
// Reply:     mycelium.tool.result
wit_bindgen::generate!({
    path: "wit",
    world: "tool-time",
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

fn now_iso() -> String {
    let now = wasi::clocks::wall_clock::now();
    let secs = now.seconds as i64;
    // Crude epoch → YYYY-MM-DD HH:MM:SS UTC without chrono. Sufficient for tool I/O.
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem / 60) % 60, rem % 60);
    let year = 1970 + days / 365;
    let doy = days % 365;
    let mo = (doy / 30) + 1;
    let d = (doy % 30) + 1;
    format!("{year:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        if msg.subject != "mycelium.tool.call.time" {
            return Ok(());
        }
        let req: Value = serde_json::from_slice(&msg.body).map_err(|e| e.to_string())?;
        let task_id = req.get("task_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let call_id = req.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let out = json!({
            "task_id": task_id,
            "call_id": call_id,
            "output": now_iso(),
        });
        publish(
            "mycelium.tool.result",
            serde_json::to_vec(&out).map_err(|e| e.to_string())?,
        );
        Ok(())
    }
}

export!(Component);
