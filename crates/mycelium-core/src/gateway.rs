//! HTTP gateway. Axum server exposing the conversation + agent CRUD that
//! the wasm gateway component used to serve.
//!
//! Routes (all JSON):
//!   GET  /health
//!   GET  /agents
//!   POST /agents
//!   GET  /agents/:id
//!   PUT  /agents/:id
//!   DELETE /agents/:id
//!   GET  /conversations?agent_id=X
//!   POST /conversations
//!   GET  /conversations/:id
//!   GET  /conversations/:id/messages
//!   POST /conversations/:id/messages
//!   DELETE /conversations/:id

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use mycelium_types::{AgentConfig, MessageRole};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::{
    agent_registry::AgentRegistry, conversation_store::ConversationStore, state::AppState,
};

#[derive(Clone)]
struct GatewayState {
    registry: AgentRegistry,
    convs: ConversationStore,
}

pub async fn run(
    state: AppState,
    registry: AgentRegistry,
    convs: ConversationStore,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    if state.config.gateway_listen.is_empty() {
        tracing::info!("gateway listen address empty; gateway disabled");
        return Ok(());
    }

    let gw_state = Arc::new(GatewayState { registry, convs });
    let app = Router::new()
        .route("/health", get(health))
        .route("/agents", get(list_agents).post(create_agent))
        .route(
            "/agents/:id",
            get(get_agent).put(update_agent).delete(delete_agent),
        )
        .route(
            "/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/conversations/:id",
            get(get_conversation).delete(delete_conversation),
        )
        .route(
            "/conversations/:id/messages",
            get(get_messages).post(append_message),
        )
        .with_state(gw_state);

    let listener = TcpListener::bind(&state.config.gateway_listen)
        .await
        .with_context(|| format!("bind gateway on {}", state.config.gateway_listen))?;
    tracing::info!(addr = %state.config.gateway_listen, "gateway listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.recv().await;
            tracing::info!("gateway draining");
        })
        .await
        .context("gateway serve")?;
    Ok(())
}

// ─────────────────────────── handlers ───────────────────────────

async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

#[derive(Deserialize)]
struct AgentReq {
    id: String,
    name: Option<String>,
    system_prompt: Option<String>,
    model: String,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default = "default_max_steps")]
    max_steps: u32,
}

fn default_max_steps() -> u32 {
    4
}

fn agent_to_json(c: &AgentConfig) -> Value {
    json!({
        "id": c.id,
        "name": c.name,
        "system_prompt": c.system_prompt,
        "model": c.model,
        "tools": c.tools,
        "max_steps": c.max_steps,
    })
}

fn agent_from_req(r: AgentReq) -> AgentConfig {
    AgentConfig {
        id: r.id.clone(),
        name: r.name.unwrap_or(r.id),
        system_prompt: r.system_prompt.unwrap_or_default(),
        model: r.model,
        tools: r.tools,
        max_steps: r.max_steps,
    }
}

async fn list_agents(State(s): State<Arc<GatewayState>>) -> impl IntoResponse {
    match s.registry.list_agents().await {
        Ok(cs) => (
            StatusCode::OK,
            Json(json!({
                "agents": cs.iter().map(agent_to_json).collect::<Vec<_>>(),
            })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn create_agent(
    State(s): State<Arc<GatewayState>>,
    Json(body): Json<AgentReq>,
) -> impl IntoResponse {
    match s.registry.create(agent_from_req(body)).await {
        Ok(c) => (StatusCode::CREATED, Json(agent_to_json(&c))),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn get_agent(
    Path(id): Path<String>,
    State(s): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    match s.registry.get(&id).await {
        Ok(Some(c)) => (StatusCode::OK, Json(agent_to_json(&c))),
        Ok(None) => err(StatusCode::NOT_FOUND, anyhow::anyhow!("agent {id} not found")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn update_agent(
    Path(id): Path<String>,
    State(s): State<Arc<GatewayState>>,
    Json(mut body): Json<AgentReq>,
) -> impl IntoResponse {
    body.id = id;
    match s.registry.update(agent_from_req(body)).await {
        Ok(c) => (StatusCode::OK, Json(agent_to_json(&c))),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn delete_agent(
    Path(id): Path<String>,
    State(s): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    match s.registry.delete(&id).await {
        Ok(()) => (StatusCode::NO_CONTENT, Json(json!({}))),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
struct CreateConvReq {
    agent_id: String,
    #[serde(default)]
    title: Option<String>,
}

async fn create_conversation(
    State(s): State<Arc<GatewayState>>,
    Json(body): Json<CreateConvReq>,
) -> impl IntoResponse {
    match s.convs.create(body.agent_id, body.title).await {
        Ok(c) => (
            StatusCode::CREATED,
            Json(json!({
                "id": c.id,
                "agent_id": c.agent_id,
                "title": c.title,
                "created_at": c.created_at,
                "updated_at": c.updated_at,
            })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
struct ListConvQ {
    agent_id: String,
}

async fn list_conversations(
    State(s): State<Arc<GatewayState>>,
    Query(q): Query<ListConvQ>,
) -> impl IntoResponse {
    match s.convs.list_for_agent(&q.agent_id).await {
        Ok(cs) => (
            StatusCode::OK,
            Json(json!({
                "conversations": cs.iter().map(|c| json!({
                    "id": c.id,
                    "agent_id": c.agent_id,
                    "title": c.title,
                    "created_at": c.created_at,
                    "updated_at": c.updated_at,
                })).collect::<Vec<_>>(),
            })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn get_conversation(
    Path(id): Path<String>,
    State(s): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    match s.convs.get(&id).await {
        Ok(Some(c)) => (
            StatusCode::OK,
            Json(json!({
                "id": c.id,
                "agent_id": c.agent_id,
                "title": c.title,
                "created_at": c.created_at,
                "updated_at": c.updated_at,
            })),
        ),
        Ok(None) => err(StatusCode::NOT_FOUND, anyhow::anyhow!("not found")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn delete_conversation(
    Path(id): Path<String>,
    State(s): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    match s.convs.delete(&id).await {
        Ok(()) => (StatusCode::NO_CONTENT, Json(json!({}))),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
struct AppendMsgReq {
    role: String,
    content: String,
    #[serde(default)]
    tool_use_id: Option<String>,
}

async fn append_message(
    Path(id): Path<String>,
    State(s): State<Arc<GatewayState>>,
    Json(body): Json<AppendMsgReq>,
) -> impl IntoResponse {
    let role = match body.role.as_str() {
        "system" => MessageRole::System,
        "assistant" => MessageRole::Assistant,
        "tool" => MessageRole::Tool,
        _ => MessageRole::User,
    };
    match s
        .convs
        .append_message(&id, role, body.content, body.tool_use_id)
        .await
    {
        Ok(m) => (
            StatusCode::CREATED,
            Json(json!({
                "id": m.id,
                "role": role_str(&m.role),
                "content": m.content,
                "tool_use_id": m.tool_use_id,
                "created_at": m.created_at,
            })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn get_messages(
    Path(id): Path<String>,
    State(s): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    match s.convs.get_messages(&id).await {
        Ok(ms) => (
            StatusCode::OK,
            Json(json!({
                "messages": ms.iter().map(|m| json!({
                    "id": m.id,
                    "role": role_str(&m.role),
                    "content": m.content,
                    "tool_use_id": m.tool_use_id,
                    "created_at": m.created_at,
                })).collect::<Vec<_>>(),
            })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// ─────────────────────────── helpers ───────────────────────────

fn role_str(r: &MessageRole) -> &'static str {
    match r {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

fn err(status: StatusCode, e: anyhow::Error) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error": e.to_string()})))
}
