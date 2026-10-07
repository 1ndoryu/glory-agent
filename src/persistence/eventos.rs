//! Eventos de auditoría (`agent_eventos`): append-only, sin editar ni borrar.

use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::AgentError;
use crate::models::AgentEvento;

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
/// `pub(crate)`: lo usa `sesiones::handoff` para auditar la transición.
pub(crate) fn actor_limpio(actor: &str) -> Result<&str, AgentError> {
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
