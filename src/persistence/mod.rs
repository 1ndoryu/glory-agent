/* Persistencia SQLx (PostgreSQL). SQL solo aquí, con binds (sin interpolar).
 * Se usa sqlx::query_as con FromRow (runtime, no el macro query_as!) a
 * propósito: la lib debe compilar sin BD viva ni DATABASE_URL en build.
 * El SQL se verifica contra la migración 0001 y el smoke E2E de F5. */

use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::AgentError;
use crate::models::{AgentConfig, ChatMessage, ChatSession, OutboxEntry, ResponseCycle};

/// Estados válidos de `agent_sessions.status` (CHECK en 0001).
#[must_use]
pub fn valid_session_status(status: &str) -> bool {
    matches!(status, "open" | "escalated" | "closed")
}

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

/// Idempotente: el transporte acepta `session_id` elegido por el cliente
/// (el widget genera UUID v4), así que la fila puede no existir. Sin esto
/// del primer mensaje falla por `FK` (hallazgo F5/smoke 169A-1). Defaults de
/// `status`/`ai_enabled` aplican; no toca filas existentes.
pub async fn ensure_session(pool: &PgPool, session_id: Uuid) -> Result<(), AgentError> {
    sqlx::query("INSERT INTO agent_sessions (id) VALUES ($1) ON CONFLICT (id) DO NOTHING")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
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

/* [169A-3] Gestión staff/config: el panel admin lista, toma/suelta la IA
 * y edita la config sin tocar código. SQL con binds; validación en el
 * boundary (los CHECK de BD son última defensa). */

pub async fn get_session(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<ChatSession>, AgentError> {
    let row = sqlx::query_as::<_, ChatSession>(
        "SELECT id, visitor_name, contact, status, ai_enabled, created_at, updated_at \
         FROM agent_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Sesiones para el panel staff, recientes primero. `status=None` = todas.
pub async fn list_sessions(
    pool: &PgPool,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<ChatSession>, AgentError> {
    if let Some(s) = status {
        if !valid_session_status(s) {
            return Err(AgentError::BadRequest(
                "session status debe ser open|escalated|closed".to_string(),
            ));
        }
    }
    let limit = limit.clamp(1, 200);
    let rows = sqlx::query_as::<_, ChatSession>(
        "SELECT id, visitor_name, contact, status, ai_enabled, created_at, updated_at \
         FROM agent_sessions WHERE ($1::TEXT IS NULL OR status = $1) \
         ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(status)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn set_session_status(
    pool: &PgPool,
    session_id: Uuid,
    status: &str,
) -> Result<ChatSession, AgentError> {
    if !valid_session_status(status) {
        return Err(AgentError::BadRequest(
            "session status debe ser open|escalated|closed".to_string(),
        ));
    }
    let row = sqlx::query_as::<_, ChatSession>(
        "UPDATE agent_sessions SET status = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
    )
    .bind(session_id)
    .bind(status)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Guarda nombre/contacto del visitante (tool `registrar_contacto` o form).
/// `None` = no tocar ese campo; strings vacíos se guardan como NULL.
pub async fn set_session_contact(
    pool: &PgPool,
    session_id: Uuid,
    visitor_name: Option<&str>,
    contact: Option<&str>,
) -> Result<ChatSession, AgentError> {
    let name = visitor_name.map(str::trim).filter(|s| !s.is_empty());
    let contact = contact.map(str::trim).filter(|s| !s.is_empty());
    let row = sqlx::query_as::<_, ChatSession>(
        "UPDATE agent_sessions SET visitor_name = COALESCE($2, visitor_name), \
         contact = COALESCE($3, contact), updated_at = now() WHERE id = $1 \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
    )
    .bind(session_id)
    .bind(name)
    .bind(contact)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Tomar (`false`: un humano atiende, la IA calla) o soltar (`true`) la IA.
pub async fn set_session_ai(
    pool: &PgPool,
    session_id: Uuid,
    enabled: bool,
) -> Result<ChatSession, AgentError> {
    let row = sqlx::query_as::<_, ChatSession>(
        "UPDATE agent_sessions SET ai_enabled = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
    )
    .bind(session_id)
    .bind(enabled)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Lee una clave de `agent_config`. `None` = no definida (aplica default).
pub async fn get_config(pool: &PgPool, key: &str) -> Result<Option<String>, AgentError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM agent_config WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0))
}

/// Crea o actualiza una clave (`key` 1..64 chars, `value` <= 8000).
pub async fn set_config(pool: &PgPool, key: &str, value: &str) -> Result<AgentConfig, AgentError> {
    if key.trim().is_empty() || key.len() > 64 || value.len() > 8000 {
        return Err(AgentError::BadRequest(
            "config key 1..64 chars, value <= 8000".to_string(),
        ));
    }
    let row = sqlx::query_as::<_, AgentConfig>(
        "INSERT INTO agent_config (key, value) VALUES ($1, $2) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now() \
         RETURNING key, value, updated_at",
    )
    .bind(key.trim())
    .bind(value)
    .fetch_one(pool)
    .await?;
    Ok(row)
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
