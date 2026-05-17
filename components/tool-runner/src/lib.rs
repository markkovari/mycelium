// Tool execution sandbox.
// Subscribes to mycelium.tool.call; dispatches to registered tool providers; publishes result to mycelium.tool.result.
wit_bindgen::generate!({
    path: "wit",
    world: "tool-runner",
    generate_all,
});

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let _ = msg;
        // TODO: deserialize ToolCallRequest
        //       look up tool in tool-registry
        //       invoke tool-provider::invoke
        //       publish ToolCallResult to mycelium.tool.result
        Ok(())
    }
}

export!(Component);
