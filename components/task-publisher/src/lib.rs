// Task publisher shim.
// Exports mycelium:task/task-submit. On submit(task) → publish JSON to mycelium.task.submit.
wit_bindgen::generate!({
    path: "wit",
    world: "task-publisher",
    generate_all,
});

use serde::Serialize;

use exports::mycelium::task::task_submit::Guest;
use mycelium::types::types::{DomainError, Task};

#[derive(Serialize)]
struct TaskJson<'a> {
    id: &'a str,
    conversation_id: &'a str,
    agent_id: &'a str,
    input: &'a str,
    created_at: &'a str,
}

struct Component;

impl Guest for Component {
    fn submit(task: Task) -> Result<(), DomainError> {
        let body = TaskJson {
            id: &task.id,
            conversation_id: &task.conversation_id,
            agent_id: &task.agent_id,
            input: &task.input,
            created_at: &task.created_at,
        };
        let bytes = serde_json::to_vec(&body).map_err(|e| DomainError::Internal(e.to_string()))?;
        wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
            subject: "mycelium.task.submit".to_string(),
            reply_to: None,
            body: bytes,
        })
        .map_err(DomainError::Backend)
    }
}

export!(Component);
