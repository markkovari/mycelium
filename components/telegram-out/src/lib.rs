// Outbound Telegram replies.
//
// Subscribes mycelium.channel.telegram.out.{chat_id} via wasmcloud:messaging/handler.
// Body: {recipient_id, text} (ChannelReply JSON).
// Calls Telegram Bot API sendMessage via wasi:http/outgoing-handler.
//
// Config (wasi:config/store):
//   telegram.bot_token — BotFather token
wit_bindgen::generate!({
    path: "wit",
    world: "telegram-out",
    generate_all,
});

use serde::Deserialize;
use serde_json::json;

fn cfg(key: &str) -> Option<String> {
    wasi::config::store::get(key).ok().flatten()
}

#[derive(Deserialize)]
struct ChannelReply {
    recipient_id: String,
    text: String,
}

fn send_telegram(token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};

    let body = json!({"chat_id": chat_id, "text": text});
    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

    let headers = Fields::new();
    headers
        .set("content-type", &[b"application/json".to_vec()])
        .map_err(|e| format!("{e:?}"))?;

    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Post)
        .map_err(|_| "set method".to_string())?;
    req.set_scheme(Some(&Scheme::Https))
        .map_err(|_| "set scheme".to_string())?;
    req.set_authority(Some("api.telegram.org"))
        .map_err(|_| "set authority".to_string())?;
    req.set_path_with_query(Some(&format!("/bot{token}/sendMessage")))
        .map_err(|_| "set path".to_string())?;

    let outgoing_body = req.body().map_err(|_| "no outgoing body".to_string())?;
    {
        let stream = outgoing_body
            .write()
            .map_err(|_| "no body write stream".to_string())?;
        for chunk in body_bytes.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .map_err(|e| format!("body write: {e:?}"))?;
        }
    }
    OutgoingBody::finish(outgoing_body, None).map_err(|e| format!("finish body: {e:?}"))?;

    let future_resp = outgoing_handler::handle(req, None).map_err(|e| format!("handle: {e:?}"))?;
    future_resp.subscribe().block();
    let _ = future_resp
        .get()
        .ok_or_else(|| "response missing".to_string())?
        .map_err(|_| "response already consumed".to_string())?
        .map_err(|e| format!("recv: {e:?}"))?;
    Ok(())
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
        send_telegram(&token, &reply.recipient_id, &reply.text)
    }
}

export!(Component);
