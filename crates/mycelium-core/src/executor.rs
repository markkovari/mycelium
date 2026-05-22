//! Task executor + openclaw-style agentic loop.
//!
//! Native rewrite of `components/executor/src/lib.rs` plus the Phase F edit
//! chain. Subscribes to `mycelium.task.submit`, `mycelium.step.tool-calls`,
//! `mycelium.tool.result`, and `mycelium.step.result`. Drives the agent loop
//! by publishing `mycelium.task.step.agent` and `mycelium.tool.call.<name>`
//! between transitions, edits a single Telegram message in place to show
//! progress (🤔 → 🔧 → 📊 → final).
//!
//! All the wash-fanout dedup machinery from the wasm version (claim_task
//! marker-CAS, agent-claim reset, channel-router seen_update) is dropped —
//! native single-instance per subject means single delivery per publish.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_nats::jetstream::kv::Store;
use chrono::SecondsFormat;
use futures_util::StreamExt;
use mycelium_types::{LifecycleEvent, LifecyclePhase, MessageRole};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::{conversation_store::ConversationStore, state::AppState, telegram::TgClient};

const STATE_BUCKET: &str = "mycelium-task-state";
const PENDING_BUCKET: &str = "mycelium-channel-pending";

const TASK_SUBMIT: &str = "mycelium.task.submit";
const STEP_AGENT: &str = "mycelium.task.step.agent";
const STEP_TOOL_CALLS: &str = "mycelium.step.tool-calls";
const TOOL_RESULT: &str = "mycelium.tool.result";
const STEP_RESULT: &str = "mycelium.step.result";

pub async fn run(
    state: AppState,
    convs: ConversationStore,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    let state_kv = state
        .js
        .get_key_value(STATE_BUCKET)
        .await
        .with_context(|| format!("open {STATE_BUCKET}"))?;
    let pending_kv = state
        .js
        .get_key_value(PENDING_BUCKET)
        .await
        .with_context(|| format!("open {PENDING_BUCKET}"))?;

    let ctx = Arc::new(ExecCtx {
        state: state.clone(),
        convs,
        state_kv,
        pending_kv,
        telegram: if state.config.telegram_bot_token.is_empty() {
            None
        } else {
            Some(TgClient::new(
                state.config.telegram_bot_token.clone(),
                state.http.clone(),
            ))
        },
    });

    let mut sub_submit = state.nats.subscribe(TASK_SUBMIT.to_string()).await?;
    let mut sub_tool_calls = state.nats.subscribe(STEP_TOOL_CALLS.to_string()).await?;
    let mut sub_tool_result = state.nats.subscribe(TOOL_RESULT.to_string()).await?;
    let mut sub_step_result = state.nats.subscribe(STEP_RESULT.to_string()).await?;

    tracing::info!("executor started");

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                tracing::info!("executor shutting down");
                break;
            }
            Some(msg) = sub_submit.next() => {
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_task_submit(&ctx2, &msg.payload).await {
                        tracing::warn!(error = %e, "task.submit handler failed");
                    }
                });
            }
            Some(msg) = sub_tool_calls.next() => {
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_step_tool_calls(&ctx2, &msg.payload).await {
                        tracing::warn!(error = %e, "step.tool-calls handler failed");
                    }
                });
            }
            Some(msg) = sub_tool_result.next() => {
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_tool_result(&ctx2, &msg.payload).await {
                        tracing::warn!(error = %e, "tool.result handler failed");
                    }
                });
            }
            Some(msg) = sub_step_result.next() => {
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_step_result(&ctx2, &msg.payload).await {
                        tracing::warn!(error = %e, "step.result handler failed");
                    }
                });
            }
        }
    }
    Ok(())
}

struct ExecCtx {
    state: AppState,
    convs: ConversationStore,
    state_kv: Store,
    pending_kv: Store,
    telegram: Option<TgClient>,
}

// ─────────────────────────── persistent state ───────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskJson {
    id: String,
    conversation_id: String,
    agent_id: String,
    input: String,
    created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ToolCallSpec {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ToolResultRec {
    id: String,
    name: String,
    output: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskState {
    task: TaskJson,
    status: String,
    output: Option<String>,
    error: Option<String>,
    #[serde(default)]
    step: u32,
    #[serde(default)]
    pending_tool_calls: Vec<ToolCallSpec>,
    #[serde(default)]
    tool_results: Vec<ToolResultRec>,
    #[serde(default)]
    progress_message_id: i64,
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    max_steps: u32,
    #[serde(default)]
    run_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingTask {
    channel: String,
    chat_id: String,
    #[serde(default)]
    conversation_id: String,
}

async fn save_state(kv: &Store, state: &TaskState) -> Result<()> {
    let json = serde_json::to_vec(state)?;
    kv.put(format!("task/{}", state.task.id), json.into()).await?;
    Ok(())
}

async fn load_state(kv: &Store, id: &str) -> Result<Option<TaskState>> {
    let Some(bytes) = kv.get(format!("task/{id}")).await? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}

async fn load_pending(kv: &Store, task_id: &str) -> Result<Option<PendingTask>> {
    let Some(bytes) = kv.get(task_id).await? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}

async fn delete_pending(kv: &Store, task_id: &str) -> Result<()> {
    kv.delete(task_id).await.ok();
    Ok(())
}

// ─────────────────────────── handler arms ───────────────────────────

async fn handle_task_submit(ctx: &ExecCtx, body: &[u8]) -> Result<()> {
    let task: TaskJson = serde_json::from_slice(body).context("decode task.submit")?;

    let (chat_id, progress_message_id) = match load_pending(&ctx.pending_kv, &task.id).await? {
        Some(p) if p.channel == "telegram" => {
            let mid = match &ctx.telegram {
                Some(tg) => tg
                    .send_message(&p.chat_id, "🤔 thinking…")
                    .await
                    .map(|m| m.message_id)
                    .unwrap_or(0),
                None => 0,
            };
            (p.chat_id, mid)
        }
        _ => (String::new(), 0),
    };

    let run_id = uuid::Uuid::now_v7().to_string();
    let state = TaskState {
        task: task.clone(),
        status: "running".into(),
        output: None,
        error: None,
        step: 0,
        pending_tool_calls: Vec::new(),
        tool_results: Vec::new(),
        progress_message_id,
        chat_id,
        max_steps: 4,
        run_id: run_id.clone(),
    };
    save_state(&ctx.state_kv, &state).await?;
    emit_lifecycle(ctx, &run_id, &task.id, LifecyclePhase::Started, 0, None).await;

    let step = json!({
        "task_id": task.id,
        "conversation_id": task.conversation_id,
        "agent_id": task.agent_id,
        "run_id": run_id,
    });
    ctx.state
        .nats
        .publish(STEP_AGENT.to_string(), serde_json::to_vec(&step)?.into())
        .await?;
    emit_lifecycle(ctx, &run_id, &task.id, LifecyclePhase::Stepping, 1, None).await;
    Ok(())
}

async fn handle_step_tool_calls(ctx: &ExecCtx, body: &[u8]) -> Result<()> {
    let req: Value = serde_json::from_slice(body).context("decode step.tool-calls")?;
    let Some(task_id) = req.get("task_id").and_then(|v| v.as_str()).map(str::to_string) else {
        return Ok(());
    };
    let Some(mut state) = load_state(&ctx.state_kv, &task_id).await? else {
        return Ok(());
    };

    let max = effective_max(&state);
    if state.step >= max {
        state.status = "failed".into();
        state.error = Some("max steps exceeded".into());
        save_state(&ctx.state_kv, &state).await?;
        progress_edit(ctx, &state, &format!("⚠️ max steps exceeded ({}/{max})", state.step)).await;
        emit_lifecycle(
            ctx,
            &state.run_id,
            &task_id,
            LifecyclePhase::Error,
            state.step,
            Some("max steps exceeded".into()),
        )
        .await;
        let res = json!({
            "task_id": task_id,
            "output": Value::Null,
            "error": "max steps exceeded",
        });
        ctx.state
            .nats
            .publish(STEP_RESULT.to_string(), serde_json::to_vec(&res)?.into())
            .await?;
        return Ok(());
    }

    let calls_v = req
        .get("calls")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut calls: Vec<ToolCallSpec> = Vec::with_capacity(calls_v.len());
    for c in &calls_v {
        let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let arguments = c
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("{}")
            .to_string();
        if !name.is_empty() {
            calls.push(ToolCallSpec { id, name, arguments });
        }
    }

    state.pending_tool_calls = calls.clone();
    state.tool_results.clear();
    state.status = "waiting_tools".into();
    state.step += 1;
    let label = if calls.len() == 1 {
        let trimmed: String = calls[0].arguments.chars().take(80).collect();
        format!("🔧 {}({})", calls[0].name, trimmed)
    } else {
        let names: Vec<String> = calls.iter().map(|c| c.name.clone()).collect();
        format!("🔧 calling {} tools: {}", calls.len(), names.join(", "))
    };
    progress_edit(
        ctx,
        &state,
        &format!("{label}\n_step {}/{max}_", state.step),
    )
    .await;
    save_state(&ctx.state_kv, &state).await?;

    for c in &calls {
        let payload = json!({
            "task_id": task_id,
            "call_id": c.id,
            "arguments": c.arguments,
            "run_id": state.run_id,
        });
        ctx.state
            .nats
            .publish(
                format!("mycelium.tool.call.{}", c.name),
                serde_json::to_vec(&payload)?.into(),
            )
            .await?;
    }
    emit_lifecycle(
        ctx,
        &state.run_id,
        &task_id,
        LifecyclePhase::ToolCalls,
        state.step,
        None,
    )
    .await;
    Ok(())
}

async fn handle_tool_result(ctx: &ExecCtx, body: &[u8]) -> Result<()> {
    let req: Value = serde_json::from_slice(body).context("decode tool.result")?;
    let Some(task_id) = req.get("task_id").and_then(|v| v.as_str()).map(str::to_string) else {
        return Ok(());
    };
    let call_id = req
        .get("call_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let Some(mut state) = load_state(&ctx.state_kv, &task_id).await? else {
        return Ok(());
    };
    let name = state
        .pending_tool_calls
        .iter()
        .find(|p| p.id == call_id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let output = req.get("output").and_then(|v| v.as_str()).map(str::to_string);
    let error = req.get("error").and_then(|v| v.as_str()).map(str::to_string);
    state.tool_results.push(ToolResultRec {
        id: call_id,
        name: name.clone(),
        output: output.clone(),
        error: error.clone(),
    });
    emit_lifecycle(
        ctx,
        &state.run_id,
        &task_id,
        LifecyclePhase::ToolResult,
        state.step,
        error.clone(),
    )
    .await;

    let max = effective_max(&state);
    let preview = match (&output, &error) {
        (Some(o), _) => {
            let s = o.replace('\n', " ");
            let trimmed: String = s.chars().take(80).collect();
            format!("{name} → {trimmed}")
        }
        (_, Some(e)) => {
            let trimmed: String = e.chars().take(80).collect();
            format!("{name} → error: {trimmed}")
        }
        _ => format!("{name} → (empty)"),
    };

    let done = state.tool_results.len() >= state.pending_tool_calls.len();
    if done {
        progress_edit(
            ctx,
            &state,
            &format!("📊 {preview}\n_step {}/{max} • thinking…_", state.step),
        )
        .await;
    } else {
        let remaining = state.pending_tool_calls.len() - state.tool_results.len();
        progress_edit(
            ctx,
            &state,
            &format!("📊 {preview}\n_waiting for {remaining} more tools…_"),
        )
        .await;
    }
    save_state(&ctx.state_kv, &state).await?;
    if !done {
        return Ok(());
    }

    // All results collected — fold them into the conversation as user-role
    // synthetic messages (Gemini's OpenAI-compat surface rejects tool-role
    // messages without a matching prior assistant.tool_calls entry), then
    // re-arm the agent step.
    if let Some(pending) = load_pending(&ctx.pending_kv, &task_id).await? {
        if !pending.conversation_id.is_empty() {
            for r in &state.tool_results {
                let content = match (&r.output, &r.error) {
                    (Some(o), _) => format!("[tool {} returned: {o}]", r.name),
                    (_, Some(e)) => format!("[tool {} failed: {e}]", r.name),
                    _ => format!("[tool {} returned: <empty>]", r.name),
                };
                let _ = ctx
                    .convs
                    .append_message(
                        &pending.conversation_id,
                        MessageRole::User,
                        content,
                        None,
                    )
                    .await;
            }
        }
    }
    state.pending_tool_calls.clear();
    state.status = "running".into();
    save_state(&ctx.state_kv, &state).await?;

    let step = json!({
        "task_id": task_id,
        "conversation_id": state.task.conversation_id,
        "agent_id": state.task.agent_id,
        "run_id": state.run_id,
    });
    ctx.state
        .nats
        .publish(STEP_AGENT.to_string(), serde_json::to_vec(&step)?.into())
        .await?;
    emit_lifecycle(
        ctx,
        &state.run_id,
        &task_id,
        LifecyclePhase::Stepping,
        state.step + 1,
        None,
    )
    .await;
    Ok(())
}

async fn handle_step_result(ctx: &ExecCtx, body: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    struct StepResultBody {
        task_id: String,
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        error: Option<String>,
    }
    let res: StepResultBody = serde_json::from_slice(body).context("decode step.result")?;
    let reply_text = res
        .output
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| res.error.clone().map(|e| format!("(error: {e})")));

    let mut run_id_for_event = String::new();
    let mut step_for_event = 0u32;
    if let Some(mut state) = load_state(&ctx.state_kv, &res.task_id).await? {
        state.status = if res.error.is_some() {
            "failed".into()
        } else {
            "done".into()
        };
        state.output = res.output.clone();
        state.error = res.error.clone();
        run_id_for_event = state.run_id.clone();
        step_for_event = state.step;
        save_state(&ctx.state_kv, &state).await?;
    }
    if !run_id_for_event.is_empty() {
        let phase = if res.error.is_some() {
            LifecyclePhase::Error
        } else {
            LifecyclePhase::Completed
        };
        emit_lifecycle(
            ctx,
            &run_id_for_event,
            &res.task_id,
            phase,
            step_for_event,
            res.error.clone(),
        )
        .await;
    }

    let Some(text) = reply_text else { return Ok(()) };
    let Some(pending) = load_pending(&ctx.pending_kv, &res.task_id).await? else {
        return Ok(());
    };

    if !pending.conversation_id.is_empty() {
        let _ = ctx
            .convs
            .append_message(
                &pending.conversation_id,
                MessageRole::Assistant,
                text.clone(),
                None,
            )
            .await;
    }

    if pending.channel == "telegram" {
        if let Some(tg) = &ctx.telegram {
            let (rpm_used, rpm_cap, rpd_used, rpd_cap) = quota_snapshot(ctx).await;
            let state_for_step = load_state(&ctx.state_kv, &res.task_id).await?;
            let (step, max) = state_for_step
                .as_ref()
                .map(|s| (s.step, effective_max(s)))
                .unwrap_or((0, 4));
            let footer = format!(
                "_step {step}/{max} • RPM {rpm_used}/{rpm_cap} • RPD {rpd_used}/{rpd_cap}_"
            );
            let body = format!("{text}\n\n{footer}");

            let edited = match state_for_step
                .as_ref()
                .filter(|s| s.progress_message_id != 0 && !s.chat_id.is_empty())
            {
                Some(s) => tg
                    .edit_message_text(&s.chat_id, s.progress_message_id, &body)
                    .await
                    .is_ok(),
                None => false,
            };
            if !edited {
                let _ = tg.send_message_markdown(&pending.chat_id, &body).await;
            }
            maybe_update_bot_status(ctx, tg, rpm_used, rpm_cap, rpd_used, rpd_cap).await;
        }
    }
    delete_pending(&ctx.pending_kv, &res.task_id).await?;
    Ok(())
}

// ─────────────────────────── helpers ───────────────────────────

fn effective_max(state: &TaskState) -> u32 {
    if state.max_steps > 0 {
        state.max_steps
    } else {
        4
    }
}

async fn progress_edit(ctx: &ExecCtx, state: &TaskState, text: &str) {
    if state.progress_message_id == 0 || state.chat_id.is_empty() {
        return;
    }
    let Some(tg) = &ctx.telegram else { return };
    if let Err(e) = tg
        .edit_message_text(&state.chat_id, state.progress_message_id, text)
        .await
    {
        tracing::debug!(error = %e, "progress edit failed (non-fatal)");
    }
}

async fn quota_snapshot(ctx: &ExecCtx) -> (u64, u64, u64, u64) {
    let now = chrono::Utc::now().timestamp();
    let minute = (now / 60) as u64;
    let day = (now / 86_400) as u64;
    let rpm_cap = ctx.state.config.llm_rpm;
    let rpd_cap = ctx.state.config.llm_rpd;
    let mut rpm_used = 0u64;
    let mut rpd_used = 0u64;

    let Ok(mut keys) = ctx.state_kv.keys().await else {
        return (0, rpm_cap, 0, rpd_cap);
    };
    let min_suffix = format!("/min/{minute}");
    let day_suffix = format!("/day/{day}");
    while let Some(key) = keys.next().await {
        let Ok(key) = key else { continue };
        if !(key.contains(&min_suffix) || key.contains(&day_suffix)) {
            continue;
        }
        let Some(bytes) = ctx.state_kv.get(&key).await.ok().flatten() else {
            continue;
        };
        let Ok(s) = std::str::from_utf8(&bytes) else { continue };
        let n = s.parse::<u64>().unwrap_or(0);
        if key.contains(&min_suffix) {
            rpm_used += n;
        }
        if key.contains(&day_suffix) {
            rpd_used += n;
        }
    }
    (rpm_used, rpm_cap, rpd_used, rpd_cap)
}

async fn maybe_update_bot_status(
    ctx: &ExecCtx,
    tg: &TgClient,
    rpm_used: u64,
    rpm_cap: u64,
    rpd_used: u64,
    rpd_cap: u64,
) {
    // Telegram rate-limits setMyShortDescription to roughly 1/min.
    let now = chrono::Utc::now().timestamp() as u64;
    let last = ctx
        .state_kv
        .get("bot-status/last")
        .await
        .ok()
        .flatten()
        .and_then(|b| std::str::from_utf8(&b).ok().map(str::to_string))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if now < last + 65 {
        return;
    }
    let txt = format!("today: {rpd_used}/{rpd_cap} calls • now: {rpm_used}/{rpm_cap} this minute");
    if tg.set_my_short_description(&txt).await.is_ok() {
        let _ = ctx
            .state_kv
            .put("bot-status/last", now.to_string().into_bytes().into())
            .await;
    }
}

#[allow(dead_code)]
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

async fn emit_lifecycle(
    ctx: &ExecCtx,
    run_id: &str,
    task_id: &str,
    phase: LifecyclePhase,
    step: u32,
    error: Option<String>,
) {
    if run_id.is_empty() {
        return;
    }
    let event = LifecycleEvent {
        run_id: run_id.to_string(),
        task_id: task_id.to_string(),
        phase,
        step,
        ts: chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        error,
    };
    let subject = format!("mycelium.run.{run_id}.lifecycle");
    match serde_json::to_vec(&event) {
        Ok(bytes) => {
            if let Err(e) = ctx.state.nats.publish(subject, bytes.into()).await {
                tracing::debug!(error = %e, "lifecycle publish failed (non-fatal)");
            }
        }
        Err(e) => tracing::debug!(error = %e, "lifecycle serialize failed"),
    }
}
