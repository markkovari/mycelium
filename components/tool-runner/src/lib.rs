// Tool execution sandbox.
// Subscribes to mycelium.tool.call; dispatches via mycelium:tool/tool-registry::execute;
// publishes ToolCallResult JSON to mycelium.tool.result.
wit_bindgen::generate!({
    path: "wit",
    world: "tool-runner",
    generate_all,
});

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct ToolCallReqJson {
    call_id: String,
    tool_id: String,
    args_json: String,
}

#[derive(Serialize, Deserialize)]
struct ToolCallResJson {
    call_id: String,
    tool_id: String,
    output_json: String,
    is_error: bool,
}

struct Component;

fn build_err(call_id: &str, tool_id: &str, msg: String) -> ToolCallResJson {
    ToolCallResJson {
        call_id: call_id.to_string(),
        tool_id: tool_id.to_string(),
        output_json: serde_json::json!({ "error": msg }).to_string(),
        is_error: true,
    }
}

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let body = msg.body;
        let req: ToolCallReqJson = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => return Err(format!("decode tool call: {e}")),
        };

        // No tool registry wired yet. Every tool call returns a clear error so
        // the caller can choose to retry or abort the agent run. Future work:
        // register a `tool-registry` component in this workload and route via
        // WIT — see `docs/user-journeys/build-researcher.md` for the loop shape.
        let result = build_err(
            &req.call_id,
            &req.tool_id,
            "no tool registry configured".to_string(),
        );

        let body = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
        let out = wasmcloud::messaging::types::BrokerMessage {
            subject: "mycelium.tool.result".to_string(),
            reply_to: None,
            body,
        };
        wasmcloud::messaging::consumer::publish(&out)
    }
}

export!(Component);
