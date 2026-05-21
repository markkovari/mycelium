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
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem / 60) % 60, rem % 60);
    // Howard Hinnant civil_from_days: epoch-days → (y, mo, d).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mo <= 2 { y + 1 } else { y };
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
