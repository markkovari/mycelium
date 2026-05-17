// CLI ↔ Telegram pairing state machine.
//
// Exports mycelium:pairing/pairing.
// Also exposes request-reply over NATS so mycelium-cli (native binary) can call it:
//   mycelium.pair.request         → request-code
//   mycelium.pair.complete        → complete
//   mycelium.pair.get-by-session  → get-by-session
//   mycelium.pair.get-by-chat     → get-by-chat
//   mycelium.pair.unpair          → unpair
//
// KV bucket: mycelium-channel-sessions
//   pair/code/{CODE}         → PendingPair JSON (TTL 5 min)
//   pair/session/{session}   → PairInfo JSON
//   pair/chat/{chat_id}      → PairInfo JSON
wit_bindgen::generate!({
    path: "wit",
    world: "session-bridge",
    generate_all,
});

struct Component;

impl exports::mycelium::pairing::pairing::Guest for Component {
    fn request_code(
        req: mycelium::pairing::pairing::PairRequest,
    ) -> Result<mycelium::pairing::pairing::PairCode, mycelium::types::types::DomainError> {
        let _ = req;
        // TODO: generate 5-char alphanumeric code via wasi:random
        //       store pair/code/{CODE} in KV with TTL
        Err(mycelium::types::types::DomainError::Internal(
            "not implemented".into(),
        ))
    }

    fn complete(
        code: String,
        chat_id: String,
    ) -> Result<mycelium::pairing::pairing::PairInfo, mycelium::types::types::DomainError> {
        let _ = (code, chat_id);
        // TODO: look up pair/code/{code} in KV
        //       create conversation via conversation-store
        //       write pair/session/{session_id} and pair/chat/{chat_id}
        //       delete pair/code/{code}
        Err(mycelium::types::types::DomainError::Internal(
            "not implemented".into(),
        ))
    }

    fn get_by_session(
        session_id: String,
    ) -> Result<Option<mycelium::pairing::pairing::PairInfo>, mycelium::types::types::DomainError>
    {
        let _ = session_id;
        Ok(None)
    }

    fn get_by_chat(
        chat_id: String,
    ) -> Result<Option<mycelium::pairing::pairing::PairInfo>, mycelium::types::types::DomainError>
    {
        let _ = chat_id;
        Ok(None)
    }

    fn unpair(session_id: String) -> Result<(), mycelium::types::types::DomainError> {
        let _ = session_id;
        Ok(())
    }
}

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let _ = msg;
        // TODO: route mycelium.pair.* subjects to the appropriate pairing:: methods above
        //       respond via msg.reply_to for request-reply pattern
        Ok(())
    }
}

export!(Component);
