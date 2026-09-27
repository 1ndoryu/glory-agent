/* Modelos del dominio conversacional. Agnósticos al producto. */

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderType {
    Client,
    Ai,
    Staff,
    System,
}

impl SenderType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Ai => "ai",
            Self::Staff => "staff",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ChatSession {
    pub id: Uuid,
    pub visitor_name: Option<String>,
    pub contact: Option<String>,
    pub status: String,
    pub ai_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ChatMessage {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sender: String,
    pub body: String,
    pub sequence_num: i64,
    /* [F0] Usage exacto del turno (Responses `usage`): solo en mensajes
     * `ai`; NULL en filas viejas y mensajes no-IA (cada producto estima). */
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResponseCycle {
    pub session_id: Uuid,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OutboxEntry {
    pub id: Uuid,
    pub kind: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// [169A-3] Fila de `agent_config`: el admin edita `prompt_extra`,
/// teléfonos o el kill-switch sin redeploy. El núcleo define las claves
/// que lee (`prompt_extra`, `ai_enabled_global`); cada producto, las suyas.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AgentConfig {
    pub key: String,
    pub value: String,
    pub updated_at: DateTime<Utc>,
}

/// Mensaje servidor→cliente por WS. `delivery`: live (nuevo) o history (reconexión).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsServerMessage {
    pub r#type: String,
    pub message: ChatMessage,
    pub delivery: String,
}

impl WsServerMessage {
    #[must_use]
    pub fn live(message: ChatMessage) -> Self {
        Self {
            r#type: "message".to_string(),
            message,
            delivery: "live".to_string(),
        }
    }

    #[must_use]
    pub fn history(message: ChatMessage) -> Self {
        Self {
            r#type: "message".to_string(),
            message,
            delivery: "history".to_string(),
        }
    }
}

/// Mensaje cliente→servidor por WS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMessage {
    Send { body: String },
    Typing { on: bool },
}
