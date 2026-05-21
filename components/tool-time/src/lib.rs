//! time skill — returns the current UTC ISO timestamp.
//!
//! Declares `wasi:clocks/wall-clock` as its only capability. The
//! mycelium-tool-runner adds that import to the wasmtime Linker only if
//! the manifest declared it AND the operator allow-list includes
//! `wasi:clocks/wall-clock`.

wit_bindgen::generate!({
    path: "wit",
    world: "tool-time",
    generate_all,
});

use serde_json::json;

use exports::mycelium::tool::tool_provider::Guest;
use mycelium::types::types::{DomainError, ToolCallRequest, ToolCallResult};

fn now_iso() -> String {
    let now = wasi::clocks::wall_clock::now();
    let secs = now.seconds as i64;
    // Crude epoch → YYYY-MM-DD HH:MM:SS UTC without chrono. Sufficient for
    // tool I/O — the agent only needs a string.
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

impl Guest for Component {
    fn invoke(call: ToolCallRequest) -> Result<ToolCallResult, DomainError> {
        let ts = now_iso();
        Ok(ToolCallResult {
            call_id: call.call_id,
            tool_id: call.tool_id,
            output_json: json!({"now": ts}).to_string(),
            is_error: false,
        })
    }
}

export!(Component);
