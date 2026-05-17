//! mycelium CLI — interactive REPL with Telegram pairing.
//!
//! Usage:
//!   lc                                        # auto-pair flow
//!   lc --agent-id my-agent                    # choose agent
//!   lc --nats-url nats://dev:4222             # remote lattice
//!   lc --skip-pairing --session-id abc123     # resume without re-pairing

use anyhow::{Context, Result};
use clap::Parser;
use mycelium_types::{ChannelMessage, PairCode, PairInfo};
use rustyline::DefaultEditor;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "lc", about = "mycelium interactive agent REPL")]
struct Args {
    #[arg(long, default_value = "nats://127.0.0.1:4222", env = "MYCELIUM_NATS_URL")]
    nats_url: String,

    #[arg(long, default_value = "default", env = "MYCELIUM_AGENT_ID")]
    agent_id: String,

    /// Resume an existing session (skips pairing prompt)
    #[arg(long)]
    session_id: Option<String>,

    /// Skip Telegram pairing entirely
    #[arg(long)]
    skip_pairing: bool,

    /// Seconds to wait for pairing before giving up (0 = wait forever)
    #[arg(long, default_value = "300")]
    pairing_timeout: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "mycelium_cli=info".into())
                .as_str(),
        )
        .init();

    let args = Args::parse();
    let session_id = args
        .session_id
        .clone()
        .unwrap_or_else(|| format!("cli-{}", Uuid::new_v4()));

    info!("Connecting to NATS at {}", args.nats_url);
    let client = async_nats::connect(&args.nats_url)
        .await
        .with_context(|| format!("failed to connect to NATS at {}", args.nats_url))?;

    // Subscribe to replies for this session before doing anything else
    let reply_subject = format!("mycelium.channel.cli.out.{}", session_id);
    let mut reply_sub = client
        .subscribe(reply_subject.clone())
        .await
        .context("failed to subscribe to reply subject")?;

    // Pairing
    let pair_info = if args.skip_pairing {
        info!("Skipping Telegram pairing (--skip-pairing)");
        None
    } else {
        match request_and_await_pair(&client, &session_id, &args.agent_id, args.pairing_timeout)
            .await
        {
            Ok(info) => {
                println!("\n✓ Paired with Telegram chat {}", &info.chat_id);
                Some(info)
            }
            Err(e) => {
                warn!("Pairing failed or timed out: {e}. Running unpaired.");
                None
            }
        }
    };

    let conv_id = pair_info.as_ref().map(|p| p.conversation_id.clone());
    let prompt = match &pair_info {
        Some(p) => format!("[{}]> ", &p.conversation_id[..8]),
        None    => "[unpaired]> ".to_string(),
    };

    // Spawn background task to print replies
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let _reply_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(msg) = reply_sub.next() => {
                    if let Ok(text) = std::str::from_utf8(&msg.payload) {
                        if let Ok(reply) = serde_json::from_str::<serde_json::Value>(text) {
                            let content = reply["text"].as_str().unwrap_or(text);
                            println!("\n\x1b[32m[agent]\x1b[0m {content}\n");
                        } else {
                            println!("\n\x1b[32m[agent]\x1b[0m {text}\n");
                        }
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }
    });

    // REPL
    println!("mycelium ready. Type your message or Ctrl-D to exit.");
    let mut rl = DefaultEditor::new().context("failed to init readline")?;
    let mut seq: u64 = 0;

    loop {
        let line = match rl.readline(&prompt) {
            Ok(l)  => l,
            Err(_) => break,
        };

        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(text);

        if text == "/quit" || text == "/exit" {
            break;
        }
        if text == "/unpair" {
            if let Some(ref p) = pair_info {
                let _ = client
                    .publish("mycelium.pair.unpair", p.session_id.as_bytes().into())
                    .await;
                println!("Unpaired.");
            }
            break;
        }

        seq += 1;
        let msg = ChannelMessage {
            channel_msg_id:  format!("{session_id}-{seq}"),
            sender_id:       session_id.clone(),
            conversation_id: conv_id.clone(),
            agent_id:        Some(args.agent_id.clone()),
            text:            text.to_string(),
            raw_json:        "{}".into(),
        };

        let payload = serde_json::to_vec(&msg).unwrap_or_default();
        if let Err(e) = client
            .publish("mycelium.channel.in", payload.into())
            .await
        {
            eprintln!("publish error: {e}");
        }

        debug!("published message seq={seq}");
    }

    let _ = shutdown_tx.send(()).await;
    println!("Goodbye.");
    Ok(())
}

/// Request a pairing code, display it, then poll until paired or timeout.
async fn request_and_await_pair(
    client: &async_nats::Client,
    session_id: &str,
    agent_id: &str,
    timeout_secs: u64,
) -> Result<PairInfo> {
    let req = serde_json::json!({
        "session_id": session_id,
        "agent_id":   agent_id,
        "expires_in": 300u32
    });

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.request("mycelium.pair.request", req.to_string().into()),
    )
    .await
    .context("pair request timed out")??;

    let code: PairCode = serde_json::from_slice(&resp.payload)
        .context("invalid PairCode response")?;

    println!();
    println!("  ╔══════════════════════════════════════════╗");
    println!("  │  Open Telegram and message @Mycelium  │");
    println!("  │  Send:  /pair {}                    │", code.code);
    println!("  ╚══════════════════════════════════════════╝");
    println!("  Waiting for pairing (Ctrl-C to skip)…");
    println!();

    let deadline = if timeout_secs == 0 {
        None
    } else {
        Some(std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs))
    };

    loop {
        if let Some(d) = deadline {
            if std::time::Instant::now() > d {
                anyhow::bail!("pairing timed out after {timeout_secs}s");
            }
        }

        let poll_resp = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            client.request("mycelium.pair.get-by-session", session_id.as_bytes().into()),
        )
        .await;

        if let Ok(Ok(r)) = poll_resp {
            let val: serde_json::Value = serde_json::from_slice(&r.payload).unwrap_or_default();
            if val.get("conversation_id").is_some() {
                let info: PairInfo = serde_json::from_value(val)
                    .context("invalid PairInfo response")?;
                return Ok(info);
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
