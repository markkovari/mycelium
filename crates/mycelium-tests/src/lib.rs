//! Integration tests against a live mycelium lattice.
//! Requires: `just up && just init-streams && just deploy-local`

#[cfg(test)]
mod integration {
    use async_nats::Client;

    async fn nats() -> Client {
        async_nats::connect("nats://127.0.0.1:4222")
            .await
            .expect("NATS must be running — run `just up` first")
    }

    #[tokio::test]
    async fn nats_reachable() {
        let client = nats().await;
        // Publish a ping and verify no error
        client
            .publish("mycelium.test.ping", "ping".into())
            .await
            .expect("publish failed");
    }

    #[tokio::test]
    async fn pairing_request_returns_code() {
        let client = nats().await;
        let req = serde_json::json!({
            "session_id": "test-session-001",
            "agent_id":   "default",
            "expires_in": 60
        });
        let resp = client
            .request("mycelium.pair.request", req.to_string().into())
            .await
            .expect("request-reply failed");

        let body: serde_json::Value = serde_json::from_slice(&resp.payload).expect("invalid JSON");

        assert!(body.get("code").is_some(), "response must contain 'code'");
    }
}
