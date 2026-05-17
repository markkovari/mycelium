// HTTP ingress component.
// Receives REST calls, publishes to mycelium.task.submit and mycelium.channel.in via wasmcloud:messaging.
// Run `just init-wit-deps` then `wash build` to compile.
wit_bindgen::generate!({
    path: "wit",
    world: "gateway",
    generate_all,
});

struct Component;

impl exports::wasi::http::incoming_handler::Guest for Component {
    fn handle(
        request: wasi::http::types::IncomingRequest,
        response_out: wasi::http::types::ResponseOutparam,
    ) {
        let _ = (request, response_out);
        // TODO: route POST /conversations → mycelium.task.submit
        //       route GET  /conversations/:id/messages → conversation-store
        //       route POST /tasks → executor orchestrator
    }
}

export!(Component);
