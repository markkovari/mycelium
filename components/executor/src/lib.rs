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

fn cfg(key: &str) -> Option<String> {
    wasi::config::store::get(key).ok().flatten()
}

#[derive(Deserialize)]
struct PendingTask {
    channel: String,
    chat_id: String,
}

fn load_pending(task_id: &str) -> Option<PendingTask> {
    let bucket = wasi::keyvalue::store::open("mycelium-channel-pending").ok()?;
    let bytes = bucket.get(task_id).ok().flatten()?;
    serde_json::from_slice(&bytes).ok()
}

fn delete_pending(task_id: &str) -> Result<(), String> {
    let bucket = wasi::keyvalue::store::open("mycelium-channel-pending")
        .map_err(|e| format!("{e:?}"))?;
    bucket.delete(task_id).map_err(|e| format!("{e:?}"))
}

fn telegram_send(token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
    let body = serde_json::json!({"chat_id": chat_id, "text": text});
    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let headers = Fields::new();
    headers
        .set("content-type", &[b"application/json".to_vec()])
        .map_err(|e| format!("{e:?}"))?;
    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Post).map_err(|_| "method".to_string())?;
    req.set_scheme(Some(&Scheme::Https)).map_err(|_| "scheme".to_string())?;
    req.set_authority(Some("api.telegram.org"))
        .map_err(|_| "authority".to_string())?;
    req.set_path_with_query(Some(&format!("/bot{token}/sendMessage")))
        .map_err(|_| "path".to_string())?;
    let outgoing = req.body().map_err(|_| "body".to_string())?;
    {
        let stream = outgoing.write().map_err(|_| "stream".to_string())?;
        for chunk in body_bytes.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    OutgoingBody::finish(outgoing, None).map_err(|e| format!("{e:?}"))?;
    let fut = outgoing_handler::handle(req, None).map_err(|e| format!("{e:?}"))?;
    fut.subscribe().block();
    let _resp = fut
        .get()
        .ok_or("no resp")?
        .map_err(|_| "consumed".to_string())?
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
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
                let reply_text = res
                    .output
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| res.error.clone().map(|e| format!("(error: {e})")));
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
                if let Some(text) = reply_text {
                    if let Some(pending) = load_pending(&res.task_id) {
                        if pending.channel == "telegram" {
                            if let Some(token) = cfg("telegram.bot_token") {
                                let _ =
                                    telegram_send(&token, &pending.chat_id, &text);
                            }
                        }
                        let _ = delete_pending(&res.task_id);
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

export!(Component);
