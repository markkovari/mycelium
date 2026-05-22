//! mcp-todo: stateful MCP todo-list component backed by wasi:keyvalue.
//!
//! Exposes four tools:
//!   * `todo-create`   — create a new pending todo item
//!   * `todo-list`     — list todos (optional status filter: pending/done/all)
//!   * `todo-complete` — mark a todo done by ID
//!   * `todo-delete`   — delete a todo by ID
//!
//! Storage: NATS KV bucket "mycelium-todos".
//! Keys: "todo/{id}" for items, "meta/next_id" for the counter.

wit_bindgen::generate!({
    path: "wit",
    world: "mcp-todo",
    generate_all,
});

use exports::mycelium::mcp::mcp_provider::{Guest, McpResult, ToolDef};
use wasi::clocks::wall_clock;
use wasi::keyvalue::store;

struct Component;

impl Guest for Component {
    fn list_tools() -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "todo-create".into(),
                description: "Create a new todo item.".into(),
                input_schema: r#"{"type":"object","properties":{"title":{"type":"string","description":"Todo title"},"description":{"type":"string","description":"Optional description"}},"required":["title"]}"#.into(),
            },
            ToolDef {
                name: "todo-list".into(),
                description: "List todos. Optional status filter: pending, done, or all (default).".into(),
                input_schema: r#"{"type":"object","properties":{"status":{"type":"string","enum":["pending","done","all"],"description":"Filter by status"}}}"#.into(),
            },
            ToolDef {
                name: "todo-complete".into(),
                description: "Mark a todo as done by its numeric ID.".into(),
                input_schema: r#"{"type":"object","properties":{"id":{"type":"string","description":"Todo ID"}},"required":["id"]}"#.into(),
            },
            ToolDef {
                name: "todo-delete".into(),
                description: "Delete a todo by its numeric ID.".into(),
                input_schema: r#"{"type":"object","properties":{"id":{"type":"string","description":"Todo ID"}},"required":["id"]}"#.into(),
            },
        ]
    }

    fn call_tool(name: String, arguments_json: String) -> Result<McpResult, String> {
        let args: serde_json::Value = serde_json::from_str(&arguments_json)
            .map_err(|e| format!("invalid JSON: {e}"))?;

        let bucket = store::open("mycelium-todos")
            .map_err(|e| format!("open bucket: {e:?}"))?;

        let result = match name.as_str() {
            "todo-create"   => todo_create(&bucket, &args),
            "todo-list"     => todo_list(&bucket, &args),
            "todo-complete" => todo_complete(&bucket, &args),
            "todo-delete"   => todo_delete(&bucket, &args),
            other           => Err(format!("unknown tool: {other}")),
        }?;

        Ok(McpResult { output_json: result, is_error: false })
    }
}

fn next_id(bucket: &store::Bucket) -> Result<u64, String> {
    let current = bucket
        .get("meta/next_id")
        .map_err(|e| format!("get next_id: {e:?}"))?
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let id = current + 1;
    let id_str = id.to_string();
    bucket
        .set("meta/next_id", id_str.as_bytes())
        .map_err(|e| format!("set next_id: {e:?}"))?;
    Ok(id)
}

fn todo_create(bucket: &store::Bucket, args: &serde_json::Value) -> Result<String, String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or("missing required field: title")?;
    let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");

    let id = next_id(bucket)?.to_string();
    let dt = wall_clock::now();
    let created_at = format!("{}.{:09}", dt.seconds, dt.nanoseconds);

    let todo = serde_json::json!({
        "id": id,
        "title": title,
        "description": description,
        "status": "pending",
        "created_at": created_at,
    });

    let todo_str = todo.to_string();
    bucket
        .set(&format!("todo/{id}"), todo_str.as_bytes())
        .map_err(|e| format!("set todo: {e:?}"))?;

    Ok(todo_str)
}

fn todo_list(bucket: &store::Bucket, args: &serde_json::Value) -> Result<String, String> {
    let filter = args.get("status").and_then(|v| v.as_str()).unwrap_or("all");

    let key_resp = bucket
        .list_keys(None)
        .map_err(|e| format!("list_keys: {e:?}"))?;

    let mut todos: Vec<serde_json::Value> = Vec::new();
    for key in &key_resp.keys {
        if !key.starts_with("todo/") {
            continue;
        }
        if let Ok(Some(bytes)) = bucket.get(key) {
            if let Ok(item) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
                let include = match filter {
                    "pending" => status == "pending",
                    "done"    => status == "done",
                    _         => true,
                };
                if include {
                    todos.push(item);
                }
            }
        }
    }

    Ok(serde_json::json!({ "todos": todos }).to_string())
}

fn todo_complete(bucket: &store::Bucket, args: &serde_json::Value) -> Result<String, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("missing required field: id")?;
    let key = format!("todo/{id}");

    let bytes = bucket
        .get(&key)
        .map_err(|e| format!("get todo: {e:?}"))?
        .ok_or_else(|| format!("todo {id} not found"))?;

    let mut item: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse todo: {e}"))?;
    item["status"] = serde_json::json!("done");

    let item_str = item.to_string();
    bucket
        .set(&key, item_str.as_bytes())
        .map_err(|e| format!("update todo: {e:?}"))?;

    Ok(item_str)
}

fn todo_delete(bucket: &store::Bucket, args: &serde_json::Value) -> Result<String, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("missing required field: id")?;

    bucket
        .delete(&format!("todo/{id}"))
        .map_err(|e| format!("delete todo: {e:?}"))?;

    Ok(serde_json::json!({ "deleted": id }).to_string())
}

export!(Component);
