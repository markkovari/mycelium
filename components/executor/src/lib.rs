// Orchestration engine.
// Subscribes to mycelium.task.submit; drives the agent step loop by publishing to mycelium.task.step.agent.
// Persists TaskState to mycelium-task-state KV bucket after every transition.
//
// Message shapes (JSON):
//   mycelium.task.submit body:  Task { id, conversation_id, agent_id, input, created_at }
//   mycelium.task.step.agent body: { task_id, conversation_id, agent_id }
//   mycelium.step.result body:  { task_id, output, error? }
wit_bindgen::generate!({
    path: "wit",
    world: "executor",
    generate_all,
});

use serde::{Deserialize, Serialize};

const BUCKET: &str = "mycelium-task-state";

#[derive(Serialize, Deserialize, Clone)]
struct TaskJson {
    id: String,
    conversation_id: String,
    agent_id: String,
    input: String,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct TaskStateJson {
    task: TaskJson,
    status: String,
    output: Option<String>,
    error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StepResult {
    task_id: String,
    output: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct StepRequest<'a> {
    task_id: &'a str,
    conversation_id: &'a str,
    agent_id: &'a str,
}

fn open() -> Result<wasi::keyvalue::store::Bucket, String> {
    wasi::keyvalue::store::open(BUCKET).map_err(|e| format!("{e:?}"))
}

fn save_state(state: &TaskStateJson) -> Result<(), String> {
    let bucket = open()?;
    let json = serde_json::to_vec(state).map_err(|e| e.to_string())?;
    bucket
        .set(&format!("task/{}", state.task.id), &json)
        .map_err(|e| format!("{e:?}"))
}

fn load_state(id: &str) -> Result<Option<TaskStateJson>, String> {
    let bucket = open()?;
    match bucket
        .get(&format!("task/{id}"))
        .map_err(|e| format!("{e:?}"))?
    {
        Some(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
        )),
        None => Ok(None),
    }
}

fn publish(subject: &str, body: Vec<u8>) -> Result<(), String> {
    wasmcloud::messaging::consumer::publish(&wasmcloud::messaging::types::BrokerMessage {
        subject: subject.to_string(),
        reply_to: None,
        body,
    })
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let body = msg.body;
        match msg.subject.as_str() {
            "mycelium.task.submit" => {
                let task: TaskJson =
                    serde_json::from_slice(&body).map_err(|e| format!("decode task: {e}"))?;

                let state = TaskStateJson {
                    task: task.clone(),
                    status: "running".into(),
                    output: None,
                    error: None,
                };
                save_state(&state)?;

                let step = StepRequest {
                    task_id: &task.id,
                    conversation_id: &task.conversation_id,
                    agent_id: &task.agent_id,
                };
                let step_body = serde_json::to_vec(&step).map_err(|e| e.to_string())?;
                publish("mycelium.task.step.agent", step_body)?;
                Ok(())
            }
            "mycelium.step.result" => {
                let res: StepResult =
                    serde_json::from_slice(&body).map_err(|e| format!("decode step: {e}"))?;
                if let Some(mut state) = load_state(&res.task_id)? {
                    state.status = if res.error.is_some() {
                        "failed"
                    } else {
                        "done"
                    }
                    .into();
                    state.output = res.output;
                    state.error = res.error;
                    save_state(&state)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

export!(Component);
