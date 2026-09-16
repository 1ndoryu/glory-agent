/* Persistencia SQLx (PostgreSQL). SQL solo aquí, con binds (sin interpolar).
 * Se usa sqlx::query_as con FromRow (runtime, no el macro query_as!) a
 * propósito: la lib debe compilar sin BD viva ni DATABASE_URL en build.
 * El SQL se verifica contra la migración 0001 y el smoke E2E de F5. */

use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::AgentError;
use crate::models::{ChatMessage, ChatSession, OutboxEntry};

pub async fn create_session(
    pool: &PgPool,
    visitor_name: Option<&str>,
    contact: Option<&str>,
) -> Result<ChatSession, AgentError> {
    let session = sqlx::query_as::<_, ChatSession>(
        "INSERT INTO agent_sessions (visitor_name, contact) VALUES ($1, $2) \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
    )
    .bind(visitor_name)
    .bind(contact)
    .fetch_one(pool)
    .await?;
    Ok(session)
}

pub async fn insert_message(
    pool: &PgPool,
    session_id: Uuid,
    sender: &str,
    body: &str,
    sequence_num: i64,
) -> Result<ChatMessage, AgentError> {
    if body.trim().is_empty() || body.len() > 8000 {
        return Err(AgentError::BadRequest(
            "mensaje vacío o >8000 chars".to_string(),
        ));
    }
    let msg = sqlx::query_as::<_, ChatMessage>(
        "INSERT INTO agent_messages (session_id, sender, body, sequence_num) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, session_id, sender, body, sequence_num, created_at",
    )
    .bind(session_id)
    .bind(sender)
    .bind(body)
    .bind(sequence_num)
    .fetch_one(pool)
    .await?;
    Ok(msg)
}

pub async fn list_messages(
    pool: &PgPool,
    session_id: Uuid,
    limit: i64,
) -> Result<Vec<ChatMessage>, AgentError> {
    let limit = limit.clamp(1, 200);
    let rows = sqlx::query_as::<_, ChatMessage>(
        "SELECT id, session_id, sender, body, sequence_num, created_at \
         FROM agent_messages WHERE session_id = $1 ORDER BY sequence_num DESC LIMIT $2",
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn enqueue_outbox(
    pool: &PgPool,
    kind: &str,
    payload: serde_json::Value,
) -> Result<OutboxEntry, AgentError> {
    let entry = sqlx::query_as::<_, OutboxEntry>(
        "INSERT INTO agent_outbox (kind, payload) VALUES ($1, $2) \
         RETURNING id, kind, payload, status, created_at",
    )
    .bind(kind)
    .bind(payload)
    .fetch_one(pool)
    .await?;
    Ok(entry)
}
