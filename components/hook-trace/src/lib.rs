//! hook-trace — minimal hook component that logs the event + payload
//! and returns the payload unchanged. Useful for smoke-testing the
//! hook-runner pipeline before installing anything mutate-y.

wit_bindgen::generate!({
    path: "wit",
    world: "hook-trace",
    generate_all,
});

use exports::mycelium::hook::hook_provider::Guest;

struct Component;

impl Guest for Component {
    fn handle(event_name: String, payload_json: String) -> Result<String, String> {
        wasi::logging::logging::log(
            wasi::logging::logging::Level::Info,
            "hook-trace",
            &format!("event={event_name} payload={payload_json}"),
        );
        Ok(payload_json)
    }
}

export!(Component);
