//! Tool schema validation and dispatch helpers.

use anyhow::{bail, Result};
use mycelium_types::{ToolCallRequest, ToolCallResult};
use serde_json::Value;

/// Minimal JSON Schema validator: checks required fields exist in args_json.
pub fn validate_args(schema_json: &str, args_json: &str) -> Result<()> {
    let schema: Value = serde_json::from_str(schema_json)?;
    let args: Value = serde_json::from_str(args_json)?;

    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        for field in required {
            if let Some(name) = field.as_str() {
                if args.get(name).is_none() {
                    bail!("missing required field: {name}");
                }
            }
        }
    }
    Ok(())
}

/// Build an error ToolCallResult from an anyhow error.
pub fn error_result(call: &ToolCallRequest, err: anyhow::Error) -> ToolCallResult {
    ToolCallResult {
        call_id: call.call_id.clone(),
        tool_id: call.tool_id.clone(),
        output_json: serde_json::json!({ "error": err.to_string() }).to_string(),
        is_error: true,
    }
}
