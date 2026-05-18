// Outbound Telegram replies.
//
// Subscribes mycelium.channel.telegram.out.{chat_id}. Payload = ChannelReply.
// Calls Telegram Bot API sendMessage via the mycelium-telegram crate.
//
// Config (wasi:config/store):
//   telegram.bot_token — BotFather token
wit_bindgen::generate!({
    path: "wit",
    world: "telegram-out",
    generate_all,
});

use mycelium_telegram::TgClient;
use serde::Deserialize;

fn cfg(key: &str) -> Option<String> {
    wasi::config::store::get(key).ok().flatten()
}

#[derive(Deserialize)]
struct ChannelReply {
    recipient_id: String,
    text: String,
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        if !msg.subject.starts_with("mycelium.channel.telegram.out.") {
            return Ok(());
        }
        let token = cfg("telegram.bot_token")
            .ok_or_else(|| "telegram.bot_token not configured".to_string())?;
        let reply: ChannelReply =
            serde_json::from_slice(&msg.body).map_err(|e| format!("decode reply: {e}"))?;
        TgClient::new(token).send_message(&reply.recipient_id, &reply.text)
    }
}

export!(Component);
