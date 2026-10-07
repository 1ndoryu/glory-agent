//! Buzón de salida (`agent_outbox`): trabajo pendiente para el consumidor.

use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::AgentError;
use crate::models::OutboxEntry;

/// Estados válidos de `agent_outbox.status` (CHECK en 0001).
#[must_use]
pub fn valid_outbox_status(status: &str) -> bool {
    matches!(status, "pending" | "sent" | "failed")
}

pub async fn enqueue_outbox(
    pool: &PgPool,
    kind: &str,
    payload: serde_json::Value,
) -> Result<OutboxEntry, AgentError> {
    let entry = sqlx::query_as!(
        OutboxEntry,
        "INSERT INTO agent_outbox (kind, payload) VALUES ($1, $2) \
         RETURNING id, kind, payload, status, created_at",
        kind,
        payload
    )
    .fetch_one(pool)
    .await?;
    Ok(entry)
}

/// Lado consumidor del outbox: pendientes más antiguos primero.
pub async fn fetch_pending_outbox(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<OutboxEntry>, AgentError> {
    let limit = limit.clamp(1, 100);
    let rows = sqlx::query_as!(
        OutboxEntry,
        "SELECT id, kind, payload, status, created_at \
         FROM agent_outbox WHERE status = 'pending' ORDER BY created_at ASC LIMIT $1",
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn mark_outbox(pool: &PgPool, id: Uuid, status: &str) -> Result<OutboxEntry, AgentError> {
    if !valid_outbox_status(status) {
        return Err(AgentError::BadRequest(
            "outbox status debe ser pending|sent|failed".to_string(),
        ));
    }
    let entry = sqlx::query_as!(
        OutboxEntry,
        "UPDATE agent_outbox SET status = $2 WHERE id = $1 \
         RETURNING id, kind, payload, status, created_at",
        id,
        status
    )
    .fetch_one(pool)
    .await?;
    Ok(entry)
}
