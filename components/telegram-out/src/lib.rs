// Outbound Telegram replies.
//
// Subscribes:
//   mycelium.channel.telegram.out.{chat_id}      Payload = ChannelReply (sendMessage)
//   mycelium.channel.telegram.action.{chat_id}   Payload = {"action": "typing"} (sendChatAction)
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

#[derive(Deserialize)]
struct ChannelAction {
    #[serde(default = "default_action")]
    action: String,
}

fn default_action() -> String {
    "typing".to_string()
}

fn extract_chat_id<'a>(subject: &'a str, prefix: &str) -> Option<&'a str> {
    subject.strip_prefix(prefix)
}

struct Component;

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(msg: wasmcloud::messaging::types::BrokerMessage) -> Result<(), String> {
        let token = cfg("telegram.bot_token")
            .ok_or_else(|| "telegram.bot_token not configured".to_string())?;
        if msg.subject.starts_with("mycelium.channel.telegram.out.") {
            let reply: ChannelReply =
                serde_json::from_slice(&msg.body).map_err(|e| format!("decode reply: {e}"))?;
            return TgClient::new(token).send_message(&reply.recipient_id, &reply.text);
        }
        if let Some(chat_id) = extract_chat_id(&msg.subject, "mycelium.channel.telegram.action.") {
            let act: ChannelAction = if msg.body.is_empty() {
                ChannelAction { action: default_action() }
            } else {
                serde_json::from_slice(&msg.body)
                    .map_err(|e| format!("decode action: {e}"))?
            };
            return TgClient::new(token).send_chat_action(chat_id, &act.action);
        }
        Ok(())
    }
}

export!(Component);
