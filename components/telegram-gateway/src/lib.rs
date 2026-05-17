// Telegram bot webhook receiver + reply sender.
//
// HTTP handler: POST /webhook → parse Telegram Update → normalise → publish mycelium.channel.in
// Messaging handler: subscribe mycelium.channel.telegram.out.{chat_id} → call Telegram Bot API
//
// Config keys (via wasi:config/store):
//   telegram.bot_token      — BotFather token
//   telegram.webhook_secret — HMAC secret for request validation
wit_bindgen::generate!({
    path: "wit",
    world: "telegram-gateway",
    generate_all,
});

struct Component;

impl exports::wasi::http::incoming_handler::Guest for Component {
    fn handle(
        request: wasi::http::types::IncomingRequest,
        response_out: wasi::http::types::ResponseOutparam,
    ) {
        let _ = (request, response_out);
        // TODO:
        // 1. validate X-Telegram-Bot-Api-Secret-Token header
        // 2. read body, parse as Telegram Update JSON
        // 3. if update.message.text starts with "/pair " → call pairing::complete
        // 4. else → call channel::inbound::normalize, publish to mycelium.channel.in
        // 5. respond 200 OK immediately (Telegram requires fast ACK)
    }
}

impl exports::wasmcloud::messaging::handler::Guest for Component {
    fn handle_message(
        msg: wasmcloud::messaging::types::BrokerMessage,
    ) -> Result<(), String> {
        let _ = msg;
        // TODO:
        // 1. deserialize ChannelReply from msg.body
        // 2. call Telegram sendMessage API via wasi:http/outgoing-handler
        //    POST https://api.telegram.org/bot{token}/sendMessage
        Ok(())
    }
}

export!(Component);
