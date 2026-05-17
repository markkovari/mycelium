// Event fan-out router.
// Subscribes to lc.event.> and fans messages out to registered target subjects.
// Rules stored in mycelium-router-rules KV bucket.
wit_bindgen::generate!({
    path: "wit",
    world: "router",
    generate_all,
});

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(
        msg: wasmcloud::messaging::types::BrokerMessage,
    ) -> Result<(), String> {
        let _ = msg;
        // TODO: load matching route-rules from KV
        //       publish msg to each target subject via wasmcloud:messaging/producer
        Ok(())
    }
}

export!(Component);
