// Conversation and message persistence.
// Exports mycelium:conversation/conversations.
// KV buckets: mycelium-conversations (metadata), mycelium-messages (per-conv message log).
// Key formats:
//   conv/{conversation_id}         → Conversation JSON
//   msg/{conversation_id}/{msg_id} → Message JSON
wit_bindgen::generate!({
    path: "wit",
    world: "conversation-store",
    generate_all,
});

struct Component;

impl exports::mycelium::conversation::conversations::Guest for Component {
    fn create(
        agent_id: String,
        title: Option<String>,
    ) -> Result<
        mycelium::types::types::Conversation,
        mycelium::types::types::DomainError,
    > {
        let _ = (agent_id, title);
        // TODO: generate UUID, write to KV, return Conversation
        Err(mycelium::types::types::DomainError::Internal(
            "not implemented".into(),
        ))
    }

    fn get(
        id: String,
    ) -> Result<
        mycelium::types::types::Conversation,
        mycelium::types::types::DomainError,
    > {
        let _ = id;
        Err(mycelium::types::types::DomainError::NotFound(
            id.clone(),
        ))
    }

    fn list(
        agent_id: String,
    ) -> Result<
        Vec<mycelium::types::types::Conversation>,
        mycelium::types::types::DomainError,
    > {
        let _ = agent_id;
        Ok(vec![])
    }

    fn delete(
        id: String,
    ) -> Result<(), mycelium::types::types::DomainError> {
        let _ = id;
        Ok(())
    }

    fn append_message(
        conversation_id: String,
        role: mycelium::types::types::MessageRole,
        content: String,
        tool_use_id: Option<String>,
    ) -> Result<mycelium::types::types::Message, mycelium::types::types::DomainError> {
        let _ = (conversation_id, role, content, tool_use_id);
        Err(mycelium::types::types::DomainError::Internal(
            "not implemented".into(),
        ))
    }

    fn get_messages(
        conversation_id: String,
    ) -> Result<Vec<mycelium::types::types::Message>, mycelium::types::types::DomainError>
    {
        let _ = conversation_id;
        Ok(vec![])
    }

    fn get_messages_after(
        conversation_id: String,
        after_id: String,
    ) -> Result<Vec<mycelium::types::types::Message>, mycelium::types::types::DomainError>
    {
        let _ = (conversation_id, after_id);
        Ok(vec![])
    }
}

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(
        msg: wasmcloud::messaging::types::BrokerMessage,
    ) -> Result<(), String> {
        let _ = msg;
        Ok(())
    }
}

export!(Component);
