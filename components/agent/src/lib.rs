// LLM step executor.
// Subscribes to lc.task.step.agent and lc.tool.result.
// Calls the configured LLM via wasi:http/outgoing-handler.
// Publishes tool-call requests to lc.tool.call or final result to lc.step.result.
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
        //       if tool_calls → publish to lc.tool.call
        //       if finish → publish to lc.step.result
        Ok(())
    }
}

export!(Component);
