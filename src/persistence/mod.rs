/* Persistencia SQLx (PostgreSQL). SQL solo aquí, con binds (sin interpolar).
 * Se usan los macros query!/query_as! (verificación en compilación) con caché
 * offline `.sqlx/` (`SQLX_OFFLINE=true` en `.cargo/config.toml`): el build no
 * necesita BD viva; `cargo sqlx prepare` regenera el caché con BD local.
 * El SQL se verifica contra las migraciones y el smoke E2E de F5. */

use sqlx::PgPool;
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::errors::AgentError;
use crate::handoff::{transicion, AccionHandoff};
use crate::models::{
    AgentConfig, AgentEvento, ChatMessage, ChatSession, OutboxEntry, ResponseCycle, ResumenUso,
};
use crate::session::ChatHub;

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
    let session = sqlx::query_as!(
        ChatSession,
        "INSERT INTO agent_sessions (visitor_name, contact) VALUES ($1, $2) \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
        visitor_name,
        contact
    )
    .fetch_one(pool)
    .await?;
    Ok(session)
}

/// Idempotente: el transporte acepta `session_id` elegido por el cliente
/// (el widget genera UUID v4), así que la fila puede no existir. Sin esto
/// del primer mensaje falla por `FK` (hallazgo F5/smoke 169A-1). Defaults de
/// `status`/`ai_enabled` aplican; no toca filas existentes.
pub async fn ensure_session(pool: &PgPool, session_id: Uuid) -> Result<(), AgentError> {
    sqlx::query!(
        "INSERT INTO agent_sessions (id) VALUES ($1) ON CONFLICT (id) DO NOTHING",
        session_id
    )
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
    insert_message_with_usage(pool, session_id, sender, body, sequence_num, None, None).await
}

/// Inserta con usage exacto del turno (F0: solo mensajes `ai`; el resto
/// pasa `None` y cada producto estima). Compat: `insert_message` delega.
pub async fn insert_message_with_usage(
    pool: &PgPool,
    session_id: Uuid,
    sender: &str,
    body: &str,
    sequence_num: i64,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
) -> Result<ChatMessage, AgentError> {
    if body.trim().is_empty() || body.len() > 8000 {
        return Err(AgentError::BadRequest(
            "mensaje vacío o >8000 chars".to_string(),
        ));
    }
    insert_message_raw(
        pool,
        session_id,
        sender,
        body,
        sequence_num,
        input_tokens,
        output_tokens,
    )
    .await
    .map_err(AgentError::from)
}

/// INSERT crudo (mismo SQL cacheado en `.sqlx/`): retorna `sqlx::Error` sin
/// convertir para que `insert_message_seq` distinga el 23505 y reintente.
async fn insert_message_raw(
    pool: &PgPool,
    session_id: Uuid,
    sender: &str,
    body: &str,
    sequence_num: i64,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
) -> Result<ChatMessage, sqlx::Error> {
    let msg = sqlx::query_as!(
        ChatMessage,
        "INSERT INTO agent_messages (session_id, sender, body, sequence_num, input_tokens, output_tokens) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, session_id, sender, body, sequence_num, input_tokens, output_tokens, created_at",
        session_id,
        sender,
        body,
        sequence_num,
        input_tokens,
        output_tokens
    )
    .fetch_one(pool)
    .await?;
    Ok(msg)
}

/// Asignador único de `sequence_num`: resiembra el hub desde el MAX de BD
/// (el hub es memoria y vuelve a 1 en cada reinicio) y reintenta ante 23505
/// (carrera entre dos escritores). Sin esto, el primer mensaje post-reinicio
/// a una sesión vieja reutiliza secuencia y devuelve 500 — hallazgo batería
/// 289A-1 2026-09-28. Reutiliza `list_messages` (query ya cacheada): no
/// requiere `cargo sqlx prepare`.
pub async fn insert_message_seq(
    pool: &PgPool,
    hub: &ChatHub,
    session_id: Uuid,
    sender: &str,
    body: &str,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
) -> Result<ChatMessage, AgentError> {
    if body.trim().is_empty() || body.len() > 8000 {
        return Err(AgentError::BadRequest(
            "mensaje vacío o >8000 chars".to_string(),
        ));
    }
    for _ in 0..5 {
        let max = list_messages(pool, session_id, 1)
            .await?
            .first()
            .map_or(0, |m| m.sequence_num);
        hub.asegurar_minimo(session_id, max + 1);
        let seq = hub.next_sequence(session_id);
        match insert_message_raw(
            pool,
            session_id,
            sender,
            body,
            seq,
            input_tokens,
            output_tokens,
        )
        .await
        {
            Ok(msg) => return Ok(msg),
            Err(e) if !es_conflicto_secuencia(&e) => return Err(AgentError::from(e)),
            Err(_) => {}
        }
    }
    Err(AgentError::Internal(format!(
        "sequence_num sin hueco tras reintentos (sesión {session_id})"
    )))
}

/// Solo el 23505 del INSERT (la otra única, `id`, es UUID v4 aleatorio).
fn es_conflicto_secuencia(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.code().as_deref() == Some("23505"))
}

pub async fn list_messages(
    pool: &PgPool,
    session_id: Uuid,
    limit: i64,
) -> Result<Vec<ChatMessage>, AgentError> {
    let limit = limit.clamp(1, 200);
    let rows = sqlx::query_as!(
        ChatMessage,
        "SELECT id, session_id, sender, body, sequence_num, input_tokens, output_tokens, created_at \
         FROM agent_messages WHERE session_id = $1 ORDER BY sequence_num DESC LIMIT $2",
        session_id,
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
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

/// Borra una sesión de prueba. CASCADE elimina mensajes y ciclo.
/// Retorna filas afectadas (0 = no existía).
pub async fn delete_session(pool: &PgPool, session_id: Uuid) -> Result<u64, AgentError> {
    let r = sqlx::query!("DELETE FROM agent_sessions WHERE id = $1", session_id)
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
    let row = sqlx::query_as!(
        ChatSession,
        "SELECT id, visitor_name, contact, status, ai_enabled, created_at, updated_at \
         FROM agent_sessions WHERE id = $1",
        session_id
    )
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
    let rows = sqlx::query_as!(
        ChatSession,
        "SELECT id, visitor_name, contact, status, ai_enabled, created_at, updated_at \
         FROM agent_sessions WHERE ($1::TEXT IS NULL OR status = $1) \
         ORDER BY updated_at DESC LIMIT $2",
        status,
        limit
    )
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
    let row = sqlx::query_as!(
        ChatSession,
        "UPDATE agent_sessions SET status = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
        session_id,
        status
    )
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
    let row = sqlx::query_as!(
        ChatSession,
        "UPDATE agent_sessions SET visitor_name = COALESCE($2, visitor_name), \
         contact = COALESCE($3, contact), updated_at = now() WHERE id = $1 \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
        session_id,
        name,
        contact
    )
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
    let row = sqlx::query_as!(
        ChatSession,
        "UPDATE agent_sessions SET ai_enabled = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
        session_id,
        enabled
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Lee una clave de `agent_config`. `None` = no definida (aplica default).
pub async fn get_config(pool: &PgPool, key: &str) -> Result<Option<String>, AgentError> {
    let row = sqlx::query!("SELECT value FROM agent_config WHERE key = $1", key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.value))
}

/// Crea o actualiza una clave (`key` 1..64 chars, `value` <= 8000).
pub async fn set_config(pool: &PgPool, key: &str, value: &str) -> Result<AgentConfig, AgentError> {
    if key.trim().is_empty() || key.len() > 64 || value.len() > 8000 {
        return Err(AgentError::BadRequest(
            "config key 1..64 chars, value <= 8000".to_string(),
        ));
    }
    let row = sqlx::query_as!(
        AgentConfig,
        "INSERT INTO agent_config (key, value) VALUES ($1, $2) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now() \
         RETURNING key, value, updated_at",
        key.trim(),
        value
    )
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
    let cycle = sqlx::query_as!(
        ResponseCycle,
        "INSERT INTO agent_response_cycles (session_id, status) VALUES ($1, $2) \
         ON CONFLICT (session_id) DO UPDATE SET status = EXCLUDED.status \
         RETURNING session_id, status, created_at",
        session_id,
        status
    )
    .fetch_one(pool)
    .await?;
    Ok(cycle)
}

pub async fn get_response_cycle(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<ResponseCycle>, AgentError> {
    let cycle = sqlx::query_as!(
        ResponseCycle,
        "SELECT session_id, status, created_at \
         FROM agent_response_cycles WHERE session_id = $1",
        session_id
    )
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

/// Metadatos de canal para `record_inbound` (agrupados para no exceder
/// el tope de aridad del lint). `numero_destino_hash` es HMAC: el número
/// en plano nunca llega aquí.
#[derive(Debug, Clone, Default)]
pub struct InboundCanal {
    pub canal: String,
    pub numero_destino_hash: Option<String>,
    pub id_externo: Option<String>,
    pub via: Option<String>,
    pub media_ref: Option<serde_json::Value>,
    pub imported: bool,
}

/// Registra un mensaje inbound de canal con dedup por (`canal`, `id_externo`).
/// Retorna `None` si ya existía esa id externa en el canal (duplicado de
/// ingress: se descarta, no se duplica). Sin `id_externo` no hay dedup.
/// El import puntual (`imported = true`) siempre trae `usage` NULL por
/// construcción (CHECK `chk_agent_messages_import_sin_usage` en 0004).
pub async fn record_inbound(
    pool: &PgPool,
    session_id: Uuid,
    sender: &str,
    body: &str,
    sequence_num: i64,
    meta: &InboundCanal,
) -> Result<Option<ChatMessage>, AgentError> {
    if body.trim().is_empty() || body.len() > 8000 {
        return Err(AgentError::BadRequest(
            "mensaje vacío o >8000 chars".to_string(),
        ));
    }
    let row = sqlx::query_as!(
        ChatMessage,
        "INSERT INTO agent_messages (session_id, sender, body, sequence_num, canal, \
          numero_destino_hash, id_externo, via, media_ref, imported) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         ON CONFLICT (canal, id_externo) WHERE id_externo IS NOT NULL DO NOTHING \
         RETURNING id, session_id, sender, body, sequence_num, input_tokens, output_tokens, created_at",
        session_id,
        sender,
        body,
        sequence_num,
        meta.canal,
        meta.numero_destino_hash,
        meta.id_externo,
        meta.via,
        meta.media_ref,
        meta.imported
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/* -------------------------------------------------- F4: consola */

/// Tipos de `agent_eventos.tipo` (CHECK en 0005). Boundary primero.
#[must_use]
pub fn valid_evento_tipo(tipo: &str) -> bool {
    matches!(
        tipo,
        "handoff.tomar"
            | "handoff.devolver"
            | "handoff.cerrar"
            | "handoff.reabrir"
            | "config.cambio"
            | "import.puntual"
    )
}

/// Valida `actor` staff (1..200 chars recortado, sin PII del visitante).
fn actor_limpio(actor: &str) -> Result<&str, AgentError> {
    let actor = actor.trim();
    if actor.is_empty() || actor.len() > 200 {
        return Err(AgentError::BadRequest(
            "actor staff 1..200 chars".to_string(),
        ));
    }
    Ok(actor)
}

/// Añade un evento de auditoría (append-only: sin editar ni borrar).
pub async fn registrar_evento(
    pool: &PgPool,
    session_id: Uuid,
    tipo: &str,
    actor: &str,
    detalle: &serde_json::Value,
) -> Result<AgentEvento, AgentError> {
    if !valid_evento_tipo(tipo) {
        return Err(AgentError::BadRequest(
            "evento tipo debe ser handoff.*|config.cambio|import.puntual".to_string(),
        ));
    }
    let row = sqlx::query_as!(
        AgentEvento,
        "INSERT INTO agent_eventos (session_id, tipo, actor, detalle) VALUES ($1, $2, $3, $4) \
         RETURNING id, session_id, tipo, actor, detalle, creado_en",
        session_id,
        tipo,
        actor_limpio(actor)?,
        detalle
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Eventos de la sesión, recientes primero (`limite` 1..200).
pub async fn listar_eventos(
    pool: &PgPool,
    session_id: Uuid,
    limite: i64,
) -> Result<Vec<AgentEvento>, AgentError> {
    let rows = sqlx::query_as!(
        AgentEvento,
        "SELECT id, session_id, tipo, actor, detalle, creado_en FROM agent_eventos \
         WHERE session_id = $1 ORDER BY creado_en DESC LIMIT $2",
        session_id,
        limite.clamp(1, 200)
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Handoff atómico: valida la transición pura, cambia `status` y audita
/// en una transacción (sin hilo a medias). `motivo` opcional al `detalle`.
pub async fn handoff(
    pool: &PgPool,
    session_id: Uuid,
    accion: &str,
    actor: &str,
    motivo: Option<&str>,
) -> Result<ChatSession, AgentError> {
    let accion = AccionHandoff::parse(accion).map_err(|e| AgentError::BadRequest(e.to_string()))?;
    let actual = get_session(pool, session_id)
        .await?
        .ok_or_else(|| AgentError::NotFound(format!("sesión {session_id}")))?;
    let destino =
        transicion(&actual.status, accion).map_err(|e| AgentError::BadRequest(e.to_string()))?;
    let mut tx = pool.begin().await?;
    let sesion = sqlx::query_as!(
        ChatSession,
        "UPDATE agent_sessions SET status = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, visitor_name, contact, status, ai_enabled, created_at, updated_at",
        session_id,
        destino
    )
    .fetch_one(&mut *tx)
    .await?;
    let detalle = match motivo.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => serde_json::json!({ "motivo": m }),
        None => serde_json::json!({}),
    };
    sqlx::query!(
        "INSERT INTO agent_eventos (session_id, tipo, actor, detalle) VALUES ($1, $2, $3, $4)",
        session_id,
        accion.tipo_evento(),
        actor_limpio(actor)?,
        detalle
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(sesion)
}

/// Uso agregado de la sesión para la consola (`desde`/`hasta` `None` = todo).
/// Solo mensajes `ai` con `usage`; el resto no suma.
pub async fn resumen_uso(
    pool: &PgPool,
    session_id: Uuid,
    desde: Option<DateTime<Utc>>,
    hasta: Option<DateTime<Utc>>,
) -> Result<ResumenUso, AgentError> {
    let row = sqlx::query!(
        "SELECT COUNT(*) AS \"mensajes_ai!\", \
         COALESCE(SUM(input_tokens), 0)::BIGINT AS \"input_tokens!\", \
         COALESCE(SUM(output_tokens), 0)::BIGINT AS \"output_tokens!\" \
         FROM agent_messages WHERE session_id = $1 AND sender = 'ai' \
         AND ($2::TIMESTAMPTZ IS NULL OR created_at >= $2) \
         AND ($3::TIMESTAMPTZ IS NULL OR created_at <= $3)",
        session_id,
        desde,
        hasta
    )
    .fetch_one(pool)
    .await?;
    Ok(ResumenUso {
        mensajes_ai: row.mensajes_ai,
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
    })
}

/// Toda la config para la consola (claves del núcleo + producto).
pub async fn list_config(pool: &PgPool) -> Result<Vec<AgentConfig>, AgentError> {
    let rows = sqlx::query_as!(
        AgentConfig,
        "SELECT key, value, updated_at FROM agent_config ORDER BY key"
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
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
