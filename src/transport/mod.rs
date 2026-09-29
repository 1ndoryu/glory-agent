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
    tools::{ToolCtx, ToolExecutor, ToolRegistry, MAX_TOOL_ITERATIONS},
};

/* [F0] Turnos previos enviados al LLM (la ventana los recorta si hace
 * falta; sin esto la IA solo veía el mensaje actual). */
pub const HISTORIAL_TURNOS: i64 = 30;

#[derive(Clone)]
pub struct AgentState {
    pub hub: ChatHub,
    pub provider: ProviderConfig,
    pub prompts: PromptConfig,
    pub timing: TimingService,
    pub tools: Arc<ToolRegistry>,
    pub executor: Option<Arc<dyn ToolExecutor>>,
    pub http: reqwest::Client,
    pub pool: Option<sqlx::PgPool>,
    /* [F0] Ventana por defecto del producto; `agent_config`
     * (`context_window_tokens`) puede subirla/bajarla por turno. */
    pub context_window_tokens: usize,
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
            executor: None,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            pool: None,
            context_window_tokens: context::MAX_CONTEXT_TOKENS,
        }
    }

    /// Ventana por defecto del producto (el `agent_config` manda por turno).
    #[must_use]
    pub fn with_context_window(self, tokens: usize) -> Self {
        Self {
            context_window_tokens: tokens
                .clamp(context::MIN_CONTEXT_TOKENS, context::LIMIT_CONTEXT_TOKENS),
            ..self
        }
    }

    #[must_use]
    pub fn with_pool(mut self, pool: sqlx::PgPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /* [169A-3] Hub compartido: el producto crea un `ChatHub` y lo pasa al
     * router visitante y a sus rutas staff, así el humano responde por el
     * mismo WS que escucha el visitante (`Clone` comparte suscriptores). */
    #[must_use]
    pub fn with_hub(mut self, hub: ChatHub) -> Self {
        self.hub = hub;
        self
    }

    /// Executor de tools del producto. Sin executor, los `function_call`
    /// del modelo se ignoran y se responde solo con el texto (degradado).
    #[must_use]
    pub fn with_executor(mut self, executor: Arc<dyn ToolExecutor>) -> Self {
        self.executor = Some(executor);
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

/* [169A-3] Gate de toma humana (antes no existía: `should_answer_with_ai`
 * estaba definida pero nadie la llamaba). `false` = el mensaje igual
 * persiste y se reenvía, pero la IA no responde: kill-switch global,
 * `ai_enabled` de la sesión o ciclo `escalated`. */
async fn ai_may_respond(pool: &sqlx::PgPool, session_id: Uuid) -> Result<bool, AgentError> {
    let global_off = persistence::get_config(pool, "ai_enabled_global")
        .await
        .is_ok_and(|v| v.as_deref() == Some("off"));
    if global_off {
        return Ok(false);
    }
    let session = persistence::get_session(pool, session_id).await?;
    if session.as_ref().is_some_and(|s| !s.ai_enabled) {
        return Ok(false);
    }
    persistence::should_answer_with_ai(pool, session_id).await
}

/* Reinyecta en `messages` los items `function_call` conocidos tal cual los
 * devolvió el modelo, ANTES de sus `function_call_output`. El Responses API
 * enlaza cada salida con su llamada por `call_id`: sin el item original en
 * el `input` responde 400 `No function call found for function call output`.
 * Solo se reinyectan llamadas registradas (las desconocidas se descartan,
 * igual que al filtrar `pending`). Retorna cuántos items anexó. */
#[must_use]
pub fn inyectar_llamadas(
    messages: &mut Vec<serde_json::Value>,
    resp: &serde_json::Value,
    tools: &ToolRegistry,
) -> usize {
    let Some(items) = resp.get("output").and_then(serde_json::Value::as_array) else {
        return 0;
    };
    let mut n = 0;
    for item in items {
        let es_llamada =
            item.get("type").and_then(serde_json::Value::as_str) == Some("function_call");
        let conocida = item
            .get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|nombre| tools.contains(nombre));
        if es_llamada && conocida {
            messages.push(item.clone());
            n += 1;
        }
    }
    n
}

/* [F0] Mapea el historial persistido al rol del LLM. `staff` entra como
 * `user` marcado (contexto humano, no salida de la IA); `system` se
 * conserva; el mensaje actual (`actual_seq`, ya persistido) se excluye
 * porque el llamador lo anexa al final. */
fn historial_para_llm(mensajes: &[ChatMessage], actual_seq: i64) -> Vec<serde_json::Value> {
    mensajes
        .iter()
        .filter(|m| m.sequence_num != actual_seq && !m.body.trim().is_empty())
        .filter_map(|m| {
            let content = if m.sender == SenderType::Staff.as_str() {
                format!("[Nota staff] {}", m.body)
            } else {
                m.body.clone()
            };
            let role = match m.sender.as_str() {
                "client" | "staff" => "user",
                "ai" => "assistant",
                "system" => "system",
                _ => return None,
            };
            Some(serde_json::json!({"role": role, "content": content}))
        })
        .collect()
}

/* [169A-3] Una vuelta provider→tools: llama al provider, ejecuta los
 * `function_call` registrados y anexa sus salidas a `messages`.
 * Retorna (texto, proseguir, usage de esta llamada): `false` cuando no
 * quedan calls pendientes o no hay executor (comportamiento v1). */
async fn provider_turn(
    state: &AgentState,
    session_id: Uuid,
    messages: &mut Vec<serde_json::Value>,
    tools_ref: Option<&serde_json::Value>,
    session_header: &str,
) -> Result<(Option<String>, bool, Option<providers::TurnUsage>), AgentError> {
    let resp = providers::call_provider(
        &state.provider,
        messages,
        tools_ref,
        ChatApiOptions::standard(),
        Some(session_header),
        &state.http,
    )
    .await
    .map_err(AgentError::Ai)?;
    let out = providers::parse_responses_output(&resp);
    let Some(executor) = &state.executor else {
        return Ok((out.text, false, out.usage));
    };
    let pending: Vec<_> = out
        .function_calls
        .into_iter()
        .filter(|c| state.tools.contains(&c.name))
        .collect();
    if pending.is_empty() {
        return Ok((out.text, false, out.usage));
    }
    /* El input del siguiente turno debe traer cada `function_call` antes de
     * su `function_call_output` (si no, el provider 400 por `call_id`). */
    let _ = inyectar_llamadas(messages, &resp, &state.tools);
    let ctx = ToolCtx::new(session_id, state.pool.clone());
    for call in pending {
        let args: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or(serde_json::json!({}));
        let output = executor
            .execute(&call.name, &args, &ctx)
            .await
            .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}));
        let output_str = serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_string());
        let mut item = serde_json::Map::new();
        item.insert(
            "type".to_string(),
            serde_json::Value::String("function_call_output".to_string()),
        );
        if let Some(call_id) = call.call_id {
            item.insert("call_id".to_string(), serde_json::Value::String(call_id));
        }
        item.insert("output".to_string(), serde_json::Value::String(output_str));
        messages.push(serde_json::Value::Object(item));
    }
    Ok((out.text, true, out.usage))
}

/// Turno IA sobre un mensaje `client` YA persistido por un canal externo
/// (`WhatsApp`: el webhook persiste + vincula por su cuenta y luego llama
/// aquí). Aplica el gate humano + loop de tools + persiste `ai` +
/// broadcast, y retorna la respuesta para que el canal la reenvíe.
/// `client_seq` es la secuencia del mensaje ya guardado: se excluye del
/// historial y se re-anexa como turno actual (igual que `process_incoming`).
/// El llamante es dueño del presupuesto por remitente
/// (`timing.check_budget`) y del envío al canal con el texto retornado.
/// Sin key responde `Ok(None)` (degradado F2); el gate humano también da
/// `None` sin error (persiste + reenvía, sin `reply`: gate 169A-3).
pub async fn responder_turno_persistido(
    state: &AgentState,
    session_id: Uuid,
    body: &str,
    client_seq: i64,
) -> Result<Option<String>, AgentError> {
    if body.trim().is_empty() || body.len() > 8000 {
        return Err(AgentError::BadRequest(
            "mensaje vacío o >8000 chars".to_string(),
        ));
    }
    if state.provider.api_key.trim().is_empty() {
        return Ok(None);
    }

    /* Gate 169A-3: humano al mando → persiste y reenvía, sin `reply`. */
    if let Some(pool) = &state.pool {
        if !ai_may_respond(pool, session_id).await? {
            return Ok(None);
        }
    }

    state
        .timing
        .ai_permits_available()
        .checked_sub(1)
        .ok_or_else(|| AgentError::RateLimited("IA saturada, reintenta".to_string()))
        .map(|_| ())?;

    let mut system = state.prompts.build_system_prompt();
    if let Some(pool) = &state.pool {
        if let Ok(Some(extra)) = persistence::get_config(pool, "prompt_extra").await {
            let extra = extra.trim();
            if !extra.is_empty() {
                system.push_str("\n\nAjustes del administrador (prevalecen):\n");
                system.push_str(&extra.chars().take(2000).collect::<String>());
            }
        }
    }
    /* [F0] Historial real + ventana efectiva (helpers abajo: la función
     * no cabe en el límite de líneas con todo inline). */
    let history = cargar_historial(state, session_id, client_seq, body).await;
    let ventana = ventana_para_turno(state).await;
    let mut messages = context::build_messages_with_budget(&system, None, history, ventana);
    let tools = if state.tools.is_empty() {
        None
    } else {
        Some(state.tools.definitions())
    };
    let tools_ref = tools.as_ref();
    let session_header = session_id.to_string();

    /* Loop F6: provider→tools→provider hasta agotar calls o iteraciones.
     * El usage se suma por llamada (el loop de tools hace varias). */
    let mut reply: Option<String> = None;
    let mut uso = providers::TurnUsage::default();
    for _ in 0..=MAX_TOOL_ITERATIONS {
        let (text, proseguir, turno) =
            provider_turn(state, session_id, &mut messages, tools_ref, &session_header).await?;
        if let Some(u) = turno {
            uso = uso.saturating_add(u);
        }
        reply = text;
        if !proseguir {
            break;
        }
    }
    let uso = (uso.input_tokens > 0 || uso.output_tokens > 0).then_some(uso);

    if let (Some(pool), Some(text)) = (&state.pool, reply.clone()) {
        match persistence::insert_message_seq(
            pool,
            &state.hub,
            session_id,
            SenderType::Ai.as_str(),
            &text,
            uso.map(|u| u.input_tokens),
            uso.map(|u| u.output_tokens),
        )
        .await
        {
            Ok(msg) => {
                let _ = state.hub.broadcast(session_id, &WsServerMessage::live(msg));
            }
            Err(e) => {
                tracing::warn!("turno IA: respuesta generada pero no persistida: {e}");
            }
        }
    }
    Ok(reply)
}

/// Guarda + gate humano + llama IA (si hay key y la IA puede responder) +
/// loop de tools + broadcast. Retorna (sequence, respuesta IA).
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

    let seq = if let Some(pool) = &state.pool {
        /* Sesión elegida por el cliente: se bootstrappea idempotente antes
         * del primer mensaje (si no, FK). La secuencia la asigna
         * `insert_message_seq` (reseed + retry 23505). */
        persistence::ensure_session(pool, session_id).await?;
        let msg = persistence::insert_message_seq(
            pool,
            &state.hub,
            session_id,
            SenderType::Client.as_str(),
            body,
            None,
            None,
        )
        .await?;
        let seq = msg.sequence_num;
        let _ = state.hub.broadcast(session_id, &WsServerMessage::live(msg));
        seq
    } else {
        // Sin BD: el fanout realtime sigue funcionando con mensaje efímero.
        let seq = state.hub.next_sequence(session_id);
        let msg = ChatMessage {
            id: Uuid::new_v4(),
            session_id,
            sender: SenderType::Client.as_str().to_string(),
            body: body.to_string(),
            sequence_num: seq,
            input_tokens: None,
            output_tokens: None,
            created_at: chrono::Utc::now(),
        };
        let _ = state.hub.broadcast(session_id, &WsServerMessage::live(msg));
        seq
    };

    responder_turno_persistido(state, session_id, body, seq)
        .await
        .map(|respuesta| (seq, respuesta))
}

/* [F0] Previos cronológicos + actual al final. `list_messages` devuelve
 * DESC: se filtra el recién insertado por `seq` y se invierte. Sin BD, cae
 * al mensaje actual (comportamiento previo). */
async fn cargar_historial(
    state: &AgentState,
    session_id: Uuid,
    seq: i64,
    body: &str,
) -> Vec<serde_json::Value> {
    let mut history = vec![];
    if let Some(pool) = &state.pool {
        if let Ok(previos) =
            persistence::list_messages(pool, session_id, HISTORIAL_TURNOS + 1).await
        {
            let mut previos = historial_para_llm(&previos, seq);
            previos.reverse();
            history.append(&mut previos);
        }
    }
    history.push(serde_json::json!({"role": "user", "content": body}));
    history
}

/* [F0] Ventana efectiva: `agent_config` manda por turno; si no, la del
 * producto; la ventana queda acotada en `ventana_efectiva`. */
async fn ventana_para_turno(state: &AgentState) -> usize {
    if let Some(pool) = &state.pool {
        let cfg = persistence::get_config(pool, "context_window_tokens")
            .await
            .ok()
            .flatten();
        context::ventana_efectiva(cfg.as_deref(), state.context_window_tokens)
    } else {
        context::ventana_efectiva(None, state.context_window_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolDefinition;

    fn registro_con(nombre: &str) -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(ToolDefinition::new(
            nombre,
            "d",
            serde_json::json!({"type": "object"}),
        ));
        r
    }

    #[test]
    fn inyecta_llamada_conocida_antes_de_su_salida() {
        let tools = registro_con("buscar_inmuebles");
        let resp = serde_json::json!({"output": [
            {"type": "message", "content": []},
            {"type": "function_call", "name": "buscar_inmuebles",
             "arguments": "{}", "call_id": "call_1"},
        ]});
        let mut messages = vec![serde_json::json!({"role": "user"})];
        assert_eq!(inyectar_llamadas(&mut messages, &resp, &tools), 1);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["type"], "function_call");
        assert_eq!(messages[1]["call_id"], "call_1");
    }

    #[test]
    fn ignora_llamada_desconocida_y_otros_tipos() {
        let tools = registro_con("buscar_inmuebles");
        let resp = serde_json::json!({"output": [
            {"type": "function_call", "name": "otra_tool",
             "arguments": "{}", "call_id": "call_x"},
            {"type": "message", "content": []},
            {"type": "reasoning", "summary": []},
        ]});
        let mut messages = vec![];
        assert_eq!(inyectar_llamadas(&mut messages, &resp, &tools), 0);
        assert!(messages.is_empty());
    }

    fn mensaje(sender: &str, body: &str, seq: i64) -> ChatMessage {
        ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            sender: sender.to_string(),
            body: body.to_string(),
            sequence_num: seq,
            input_tokens: None,
            output_tokens: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn historial_mapea_roles_y_excluye_actual() {
        let previos = vec![
            mensaje("client", "hola", 1),
            mensaje("ai", "buenas", 2),
            mensaje("staff", "prioridad alta", 3),
            mensaje("client", "busco casa", 4), // actual
        ];
        let h = historial_para_llm(&previos, 4);
        assert_eq!(h.len(), 3);
        assert_eq!(h[0]["role"], "user");
        assert_eq!(h[1]["role"], "assistant");
        assert_eq!(h[2]["role"], "user");
        assert!(h[2]["content"]
            .as_str()
            .unwrap_or("")
            .starts_with("[Nota staff]"));
    }

    #[test]
    fn historial_descarta_vacios_y_desconocidos() {
        let previos = vec![
            mensaje("client", "   ", 1),
            mensaje("raro", "x", 2),
            mensaje("client", "ok", 3),
        ];
        let h = historial_para_llm(&previos, 99);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0]["content"], "ok");
    }

    #[test]
    fn sin_output_no_anexa_nada() {
        let tools = registro_con("buscar_inmuebles");
        let mut messages = vec![];
        assert_eq!(
            inyectar_llamadas(&mut messages, &serde_json::json!({}), &tools),
            0
        );
        assert!(messages.is_empty());
    }

    /* Turno sobre mensaje ya persistido (canales externos como WhatsApp):
     * sin key no hay IA (degradado F2) y no toca BD; vacío se rechaza. */
    #[tokio::test]
    async fn turno_persistido_sin_key_no_responde() {
        let state = AgentState::new(
            ProviderConfig::opencode_go(String::new()),
            PromptConfig::new("t", "s", "r", "e"),
        );
        let r = responder_turno_persistido(&state, Uuid::new_v4(), "hola", 1)
            .await
            .unwrap();
        assert_eq!(r, None);
    }

    #[tokio::test]
    async fn turno_persistido_rechaza_vacio() {
        let state = AgentState::new(
            ProviderConfig::opencode_go(String::new()),
            PromptConfig::new("t", "s", "r", "e"),
        );
        let r = responder_turno_persistido(&state, Uuid::new_v4(), "   ", 1).await;
        assert!(matches!(r, Err(AgentError::BadRequest(_))));
    }
}
