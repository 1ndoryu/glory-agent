/* Transporte: WS visitante/staff + REST. Respuesta completa por WS (sin
 * streaming token-a-token en v1). Persistencia opcional: si hay pool se guarda. */

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    context,
    errors::AgentError,
    models::{ChatMessage, SenderType, WsClientMessage, WsServerMessage},
    persistence,
    prompts::PromptConfig,
    providers::{self, ChatApiOptions, ProviderConfig},
    session::ChatHub,
    timing::TimingService,
    tools::ToolRegistry,
};

#[derive(Clone)]
pub struct AgentState {
    pub hub: ChatHub,
    pub provider: ProviderConfig,
    pub prompts: PromptConfig,
    pub timing: TimingService,
    pub tools: Arc<ToolRegistry>,
    pub http: reqwest::Client,
    pub pool: Option<sqlx::PgPool>,
}

impl AgentState {
    #[must_use]
    pub fn new(provider: ProviderConfig, prompts: PromptConfig) -> Self {
        Self {
            hub: ChatHub::new(),
            provider,
            prompts,
            timing: TimingService::new(),
            tools: Arc::new(ToolRegistry::new()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            pool: None,
        }
    }

    #[must_use]
    pub fn with_pool(mut self, pool: sqlx::PgPool) -> Self {
        self.pool = Some(pool);
        self
    }
}

#[derive(Debug, Deserialize)]
pub struct WsParams {
    pub session_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct SendBody {
    pub session_id: Uuid,
    pub body: String,
    pub visitor_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub ok: bool,
    pub sequence_num: i64,
    pub reply: Option<String>,
}

pub fn routes() -> Router<AgentState> {
    Router::new()
        .route("/agent/ws", get(ws_agent))
        .route("/agent/messages", post(rest_send))
        .route("/agent/history", get(rest_history))
}

async fn ws_agent(
    ws: WebSocketUpgrade,
    Query(params): Query<WsParams>,
    State(state): State<AgentState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, params.session_id, state))
}

async fn handle_socket(socket: WebSocket, session_id: Uuid, state: AgentState) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    state.hub.subscribe(session_id, tx);

    // Historial reciente por el socket (best-effort).
    if let Some(pool) = &state.pool {
        if let Ok(msgs) = persistence::list_messages(pool, session_id, 30).await {
            for msg in msgs.into_iter().rev() {
                let server = WsServerMessage::history(msg);
                if let Ok(json) = serde_json::to_string(&server) {
                    let _ = sender.send(Message::Text(json)).await;
                }
            }
        }
    }

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(Message::Text(text))) = receiver.next().await {
        let Ok(client_msg) = serde_json::from_str::<WsClientMessage>(&text) else {
            continue;
        };
        if let WsClientMessage::Send { body } = client_msg {
            let _ = process_incoming(&state, session_id, &body, &session_id.to_string()).await;
        }
    }
    send_task.abort();
}

async fn rest_send(
    State(state): State<AgentState>,
    Json(input): Json<SendBody>,
) -> Result<Json<SendResponse>, AgentError> {
    let key = input
        .visitor_key
        .clone()
        .unwrap_or_else(|| "anon".to_string());
    // Sin pre-chequeo aquí: process_incoming aplica el presupuesto una sola vez.
    let reply = process_incoming(&state, input.session_id, &input.body, &key).await?;
    Ok(Json(SendResponse {
        ok: true,
        sequence_num: reply.0,
        reply: reply.1,
    }))
}

#[derive(Debug, Deserialize)]
pub struct HistoryParams {
    pub session_id: Uuid,
    pub limit: Option<i64>,
}

async fn rest_history(
    State(state): State<AgentState>,
    Query(params): Query<HistoryParams>,
) -> Result<Json<Vec<ChatMessage>>, AgentError> {
    let Some(pool) = &state.pool else {
        return Ok(Json(vec![]));
    };
    let msgs =
        persistence::list_messages(pool, params.session_id, params.limit.unwrap_or(50)).await?;
    Ok(Json(msgs))
}

/// Guarda + llama IA (si hay key) + broadcast. Retorna (sequence, respuesta IA).
async fn process_incoming(
    state: &AgentState,
    session_id: Uuid,
    body: &str,
    budget_key: &str,
) -> Result<(i64, Option<String>), AgentError> {
    if body.trim().is_empty() || body.len() > 8000 {
        return Err(AgentError::BadRequest(
            "mensaje vacío o >8000 chars".to_string(),
        ));
    }
    if !state.timing.check_budget(budget_key) {
        return Err(AgentError::RateLimited("demasiadas peticiones".to_string()));
    }
    let seq = state.hub.next_sequence(session_id);

    if let Some(pool) = &state.pool {
        let msg =
            persistence::insert_message(pool, session_id, SenderType::Client.as_str(), body, seq)
                .await?;
        let _ = state.hub.broadcast(session_id, &WsServerMessage::live(msg));
    } else {
        // Sin BD: el fanout realtime sigue funcionando con mensaje efímero.
        let msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id,
            sender: SenderType::Client.as_str().to_string(),
            body: body.to_string(),
            sequence_num: seq,
            created_at: chrono::Utc::now(),
        };
        let _ = state.hub.broadcast(session_id, &WsServerMessage::live(msg));
    }

    if state.provider.api_key.trim().is_empty() {
        return Ok((seq, None));
    }

    state
        .timing
        .ai_permits_available()
        .checked_sub(1)
        .ok_or_else(|| AgentError::RateLimited("IA saturada, reintenta".to_string()))
        .map(|_| ())?;

    let system = state.prompts.build_system_prompt();
    let history = vec![serde_json::json!({"role": "user", "content": body})];
    let messages = context::build_messages(&system, None, history);
    let tools = if state.tools.is_empty() {
        None
    } else {
        Some(state.tools.definitions())
    };
    let tools_ref = tools.as_ref();
    let resp = providers::call_provider(
        &state.provider,
        &messages,
        tools_ref,
        ChatApiOptions::standard(),
        &state.http,
    )
    .await
    .map_err(AgentError::Ai)?;
    let reply = providers::extract_first_text(&resp);

    if let (Some(pool), Some(text)) = (&state.pool, reply.clone()) {
        let ai_seq = state.hub.next_sequence(session_id);
        if let Ok(msg) =
            persistence::insert_message(pool, session_id, SenderType::Ai.as_str(), &text, ai_seq)
                .await
        {
            let _ = state.hub.broadcast(session_id, &WsServerMessage::live(msg));
        }
    }
    Ok((seq, reply))
}
