// Durable event journal.
// Subscribes to lc.event.> via a JetStream durable consumer.
// Writes every event to mycelium-events-journal KV bucket for long-term audit.
wit_bindgen::generate!({
    path: "wit",
    world: "event-logger",
    generate_all,
});

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(
        msg: wasmcloud::messaging::types::BrokerMessage,
    ) -> Result<(), String> {
        let _ = msg;
        // TODO: write msg.body to mycelium-events-journal KV
        //       key = events/{subject}/{timestamp_nanos}
        Ok(())
    }
}

export!(Component);
