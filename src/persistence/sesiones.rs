//! Sesiones y mensajes (`agent_sessions`, `agent_messages`).
//!
//! Agregado de sesión: CRUD de sesiones, mensajes con `sequence_num` único
//! (resiembra + reintento 23505), `handoff` atómico y `resumen_uso`.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::AgentError;
use crate::handoff::{transicion, AccionHandoff};
use crate::models::{ChatMessage, ChatSession, ResumenUso};
use crate::session::ChatHub;

use super::eventos::actor_limpio;

/// Estados válidos de `agent_sessions.status` (CHECK en 0001).
#[must_use]
pub fn valid_session_status(status: &str) -> bool {
    matches!(status, "open" | "escalated" | "closed")
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

/// Handoff atómico: valida la transición pura, cambia `status` y audita
/// en una transacción (sin hilo a medias). `motivo` opcional al `detalle`.
///
/// La lectura previa y la apertura de la transacción son independientes y
/// van con `join!` (la regla `sqlite-carga-N-consultas` solo acepta awaits
/// sueltos si son independientes; lo que corre DENTRO de la tx —UPDATE,
/// INSERT, commit— es secuencia obligada, no paralelizable).
pub async fn handoff(
    pool: &PgPool,
    session_id: Uuid,
    accion: &str,
    actor: &str,
    motivo: Option<&str>,
) -> Result<ChatSession, AgentError> {
    let accion = AccionHandoff::parse(accion).map_err(|e| AgentError::BadRequest(e.to_string()))?;
    let (actual, tx) = tokio::join!(get_session(pool, session_id), pool.begin());
    let actual = actual?.ok_or_else(|| AgentError::NotFound(format!("sesión {session_id}")))?;
    let mut tx = tx.map_err(AgentError::from)?;
    let destino =
        transicion(&actual.status, accion).map_err(|e| AgentError::BadRequest(e.to_string()))?;
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
