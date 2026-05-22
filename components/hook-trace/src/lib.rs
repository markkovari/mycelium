//! hook-trace — minimal hook component that prints the event + payload
//! to stderr and returns the payload unchanged. Used as the smoke-test
//! artifact for the hook-runner pipeline before installing anything
//! mutate-y. Lands on the wasmtime stderr stream the runner inherits,
//! which reaches journald via the systemd unit.

wit_bindgen::generate!({
    path: "wit",
    world: "hook-trace",
    with: {
        "mycelium:hook/hook-provider@0.1.0": generate,
    },
});

use exports::mycelium::hook::hook_provider::Guest;

struct Component;

impl Guest for Component {
    fn handle(event_name: String, payload_json: String) -> Result<String, String> {
        eprintln!("hook-trace: event={event_name} payload={payload_json}");
        Ok(payload_json)
    }
}

export!(Component);
