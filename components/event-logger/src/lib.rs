// Durable event journal.
// Subscribes to mycelium.event.> via a JetStream durable consumer.
// Writes every event to mycelium-events-journal KV bucket for long-term audit.
wit_bindgen::generate!({
    path: "wit",
    world: "event-logger",
    generate_all,
});

const BUCKET: &str = "mycelium-events-journal";

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let body = msg.body;
        let now = wasi::clocks::wall_clock::now();
        let stamp = now.seconds * 1_000_000_000 + now.nanoseconds as u64;
        let key = format!("events/{}/{stamp:020}", msg.subject);
        let bucket = wasi::keyvalue::store::open(BUCKET).map_err(|e| format!("{e:?}"))?;
        bucket.set(&key, &body).map_err(|e| format!("{e:?}"))?;
        Ok(())
    }
}

export!(Component);
