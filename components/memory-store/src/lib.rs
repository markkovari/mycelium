// Agent memory (KV-backed).
// Exports mycelium:memory/memory — called directly by executor and agent components.
// Also subscribes to mycelium.memory.> for request-reply access from native clients.
// KV bucket: mycelium-memory  key format: memory/{agent_id}/{key}
wit_bindgen::generate!({
    path: "wit",
    world: "memory-store",
    generate_all,
});

struct Component;

impl exports::mycelium::memory::memory::Guest for Component {
    fn set(
        agent_id: String,
        key: String,
        value: String,
        ttl_secs: Option<u64>,
    ) -> Result<(), mycelium::types::types::DomainError> {
        let _ = (agent_id, key, value, ttl_secs);
        // TODO: wasi::keyvalue::store::open("mycelium-memory")?.set(...)
        Ok(())
    }

    fn get(
        agent_id: String,
        key: String,
    ) -> Result<Option<String>, mycelium::types::types::DomainError> {
        let _ = (agent_id, key);
        Ok(None)
    }

    fn delete(agent_id: String, key: String) -> Result<(), mycelium::types::types::DomainError> {
        let _ = (agent_id, key);
        Ok(())
    }

    fn list_keys(agent_id: String) -> Result<Vec<String>, mycelium::types::types::DomainError> {
        let _ = agent_id;
        Ok(vec![])
    }

    fn set_many(
        agent_id: String,
        entries: Vec<(String, String)>,
    ) -> Result<(), mycelium::types::types::DomainError> {
        let _ = (agent_id, entries);
        Ok(())
    }
}

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let _ = msg;
        // TODO: handle mycelium.memory.> request-reply for native clients
        Ok(())
    }
}

export!(Component);
