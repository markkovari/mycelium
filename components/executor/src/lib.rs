// Orchestration engine.
// Subscribes to mycelium.task.submit; drives the agent step loop by publishing to mycelium.task.step.agent.
// Persists TaskState to mycelium-task-state KV bucket after every transition.
wit_bindgen::generate!({
    path: "wit",
    world: "executor",
    generate_all,
});

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let _ = msg;
        // TODO: deserialize Task from msg.body
        //       write initial TaskState → KV
        //       publish step request to mycelium.task.step.agent
        Ok(())
    }
}

export!(Component);
