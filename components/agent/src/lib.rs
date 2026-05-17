// LLM step executor.
// Subscribes to mycelium.task.step.agent and mycelium.tool.result.
// Calls the configured LLM via wasi:http/outgoing-handler.
// Publishes tool-call requests to mycelium.tool.call or final result to mycelium.step.result.
wit_bindgen::generate!({
    path: "wit",
    world: "agent",
    generate_all,
});

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(
        msg: wasmcloud::messaging::types::BrokerMessage,
    ) -> Result<(), String> {
        let _ = msg;
        // TODO: read agent-config from KV
        //       build LLM message list via agent-step::build-messages
        //       call LLM via wasi:http/outgoing-handler (Ollama / Anthropic / OpenAI-compat)
        //       if tool_calls → publish to mycelium.tool.call
        //       if finish → publish to mycelium.step.result
        Ok(())
    }
}

export!(Component);
