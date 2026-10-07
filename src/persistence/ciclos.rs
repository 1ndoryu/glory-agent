//! Ciclos de respuesta (`agent_response_cycles`): toma humana de la IA.

use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::AgentError;
use crate::models::ResponseCycle;

/// Estados válidos de `agent_response_cycles.status` (CHECK en 0001).
/// Se valida en el boundary: el CHECK de BD es última defensa, no la primera.
#[must_use]
pub fn valid_cycle_status(status: &str) -> bool {
    matches!(status, "waiting" | "answered" | "escalated")
}

/// Decisión pura de toma humana: la IA responde salvo ciclo escalado.
/// Sin ciclo (`None`, sesión recién creada) la IA responde por defecto.
#[must_use]
pub fn ai_may_answer(cycle_status: Option<&str>) -> bool {
    !matches!(cycle_status, Some("escalated"))
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
