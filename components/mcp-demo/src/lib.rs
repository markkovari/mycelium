//! mcp-demo: minimal MCP server component for integration testing.
//!
//! Exposes two tools:
//!   * `text-upper`   — converts text to uppercase
//!   * `text-reverse` — reverses a string
//!
//! Invoke via:
//!   nats pub mycelium.mcp.install '{"kind":"mcp-server","name":"mcp-demo","version":"0.1.0","source":{"type":"file","path":"/path/to/mcp_demo.wasm"}}'

wit_bindgen::generate!({
    path: "wit",
    world: "mcp-demo",
    generate_all,
});

use exports::mycelium::mcp::mcp_provider::{Guest, McpResult, ToolDef};

struct Component;

impl Guest for Component {
    fn list_tools() -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "text-upper".into(),
                description: "Convert text to uppercase.".into(),
                input_schema: r#"{"type":"object","properties":{"text":{"type":"string","description":"Text to uppercase"}},"required":["text"]}"#.into(),
            },
            ToolDef {
                name: "text-reverse".into(),
                description: "Reverse a string character by character.".into(),
                input_schema: r#"{"type":"object","properties":{"text":{"type":"string","description":"Text to reverse"}},"required":["text"]}"#.into(),
            },
        ]
    }

    fn call_tool(name: String, arguments_json: String) -> Result<McpResult, String> {
        let args: serde_json::Value = serde_json::from_str(&arguments_json)
            .map_err(|e| format!("invalid arguments JSON: {e}"))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field: text".to_string())?;

        let result = match name.as_str() {
            "text-upper"   => text.to_uppercase(),
            "text-reverse" => text.chars().rev().collect(),
            other          => return Err(format!("unknown tool: {other}")),
        };

        Ok(McpResult {
            output_json: serde_json::json!({"result": result}).to_string(),
            is_error: false,
        })
    }
}

export!(Component);
