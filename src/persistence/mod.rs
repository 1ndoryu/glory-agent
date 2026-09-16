/* Persistencia SQLx (PostgreSQL). SQL solo aquí, con binds (sin interpolar).
 * Se usa sqlx::query_as con FromRow (runtime, no el macro query_as!) a
 * propósito: la lib debe compilar sin BD viva ni DATABASE_URL en build.
 * El SQL se verifica contra la migración 0001 y el smoke E2E de F5. */

use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::AgentError;
use crate::models::{ChatMessage, ChatSession, OutboxEntry, ResponseCycle};

/// Estados válidos de `agent_response_cycles.status` (CHECK en 0001).
/// Se valida en el boundary: el CHECK de BD es última defensa, no la primera.
#[must_use]
pub fn valid_cycle_status(status: &str) -> bool {
    matches!(status, "waiting" | "answered" | "escalated")
}

/// Estados válidos de `agent_outbox.status` (CHECK en 0001).
#[must_use]
pub fn valid_outbox_status(status: &str) -> bool {
    matches!(status, "pending" | "sent" | "failed")
}

/// Decisión pura de toma humana: la IA responde salvo ciclo escalado.
/// Sin ciclo (`None`, sesión recién creada) la IA responde por defecto.
#[must_use]
pub fn ai_may_answer(cycle_status: Option<&str>) -> bool {
    !matches!(cycle_status, Some("escalated"))
}

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

/// Borra una sesión de prueba. CASCADE elimina mensajes y ciclo.
/// Retorna filas afectadas (0 = no existía).
pub async fn delete_session(pool: &PgPool, session_id: Uuid) -> Result<u64, AgentError> {
    let r = sqlx::query("DELETE FROM agent_sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

/// Toma humana: crea o actualiza el ciclo de respuesta de la sesión.
/// `escalated` = un humano tomó el hilo; la IA deja de responder.
pub async fn upsert_response_cycle(
    pool: &PgPool,
    session_id: Uuid,
    status: &str,
) -> Result<ResponseCycle, AgentError> {
    if !valid_cycle_status(status) {
        return Err(AgentError::BadRequest(
            "cycle status debe ser waiting|answered|escalated".to_string(),
        ));
    }
    let cycle = sqlx::query_as::<_, ResponseCycle>(
        "INSERT INTO agent_response_cycles (session_id, status) VALUES ($1, $2) \
         ON CONFLICT (session_id) DO UPDATE SET status = EXCLUDED.status \
         RETURNING session_id, status, created_at",
    )
    .bind(session_id)
    .bind(status)
    .fetch_one(pool)
    .await?;
    Ok(cycle)
}

pub async fn get_response_cycle(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<ResponseCycle>, AgentError> {
    let cycle = sqlx::query_as::<_, ResponseCycle>(
        "SELECT session_id, status, created_at \
         FROM agent_response_cycles WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(cycle)
}

/// ¿Debe responder la IA en esta sesión? Consulta el ciclo; sin ciclo, sí.
pub async fn should_answer_with_ai(pool: &PgPool, session_id: Uuid) -> Result<bool, AgentError> {
    let cycle = get_response_cycle(pool, session_id).await?;
    Ok(ai_may_answer(cycle.as_ref().map(|c| c.status.as_str())))
}

/// Lado consumidor del outbox: pendientes más antiguos primero.
pub async fn fetch_pending_outbox(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<OutboxEntry>, AgentError> {
    let limit = limit.clamp(1, 100);
    let rows = sqlx::query_as::<_, OutboxEntry>(
        "SELECT id, kind, payload, status, created_at \
         FROM agent_outbox WHERE status = 'pending' ORDER BY created_at ASC LIMIT $1",
    )
    .bind(limit)
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
    let entry = sqlx::query_as::<_, OutboxEntry>(
        "UPDATE agent_outbox SET status = $2 WHERE id = $1 \
         RETURNING id, kind, payload, status, created_at",
    )
    .bind(id)
    .bind(status)
    .fetch_one(pool)
    .await?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::{ai_may_answer, valid_cycle_status, valid_outbox_status};

    #[test]
    fn cycle_status_rejects_unknown() {
        assert!(valid_cycle_status("waiting"));
        assert!(valid_cycle_status("answered"));
        assert!(valid_cycle_status("escalated"));
        assert!(!valid_cycle_status("open"));
        assert!(!valid_cycle_status(""));
    }

    #[test]
    fn outbox_status_rejects_unknown() {
        assert!(valid_outbox_status("pending"));
        assert!(valid_outbox_status("sent"));
        assert!(valid_outbox_status("failed"));
        assert!(!valid_outbox_status("done"));
    }

    #[test]
    fn ai_answers_unless_escalated() {
        assert!(ai_may_answer(None));
        assert!(ai_may_answer(Some("waiting")));
        assert!(ai_may_answer(Some("answered")));
        assert!(!ai_may_answer(Some("escalated")));
    }
}
