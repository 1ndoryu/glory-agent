/* Worker del outbox (plan §6/§7 F2): drena `agent_outbox` con `kind = "send"`
 * cuyo `payload` es un `Outbound` serializado. Fuente única de envío nueva
 * (endurecimiento F5): el adapter solo publica aquí; el worker es quien llama
 * al `Sender`. La `idempotency_key` la pone el caller (HMAC en bin/adapter);
 * el núcleo solo la exige única. Sin reintento automático: el fallo queda
 * `failed` y visible; reintentar es re-encolar (decisión operador/F5). */

use sqlx::PgPool;

use crate::channels::{Outbound, Sender};
use crate::errors::AgentError;
use crate::models::OutboxEntry;
use crate::persistence::{fetch_pending_outbox, mark_outbox};

/// Resultado de un pase de drenado.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrainStats {
    pub sent: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// Encola un envío de forma idempotente: si ya existe la `idempotency_key`,
/// devuelve la fila existente sin duplicar. Requiere clave no vacía.
pub async fn enqueue_send(pool: &PgPool, out: &Outbound) -> Result<OutboxEntry, AgentError> {
    out.validar()?;
    let key = out.idempotency_key.trim();
    if key.is_empty() {
        return Err(AgentError::BadRequest("idempotency_key vacía".to_string()));
    }
    let payload = serde_json::to_value(out)
        .map_err(|e| AgentError::Internal(format!("serializa Outbound: {e}")))?;
    if let Some(existente) = buscar_por_clave(pool, key).await? {
        return Ok(existente);
    }
    let fila = sqlx::query_as!(
        OutboxEntry,
        "INSERT INTO agent_outbox (kind, payload, idempotency_key) VALUES ('send', $1, $2) \
         ON CONFLICT (idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING \
         RETURNING id, kind, payload, status, created_at",
        payload,
        key
    )
    .fetch_optional(pool)
    .await?;
    if let Some(fila) = fila {
        return Ok(fila);
    }
    /* Carrera: otro escritor ganó el conflicto entre el SELECT y el INSERT. */
    buscar_por_clave(pool, key).await?.ok_or_else(|| {
        AgentError::Internal("outbox idempotente sin fila tras conflicto".to_string())
    })
}

async fn buscar_por_clave(pool: &PgPool, key: &str) -> Result<Option<OutboxEntry>, AgentError> {
    let fila = sqlx::query_as!(
        OutboxEntry,
        "SELECT id, kind, payload, status, created_at \
         FROM agent_outbox WHERE idempotency_key = $1",
        key
    )
    .fetch_optional(pool)
    .await?;
    Ok(fila)
}

/// Un pase de drenado: `send` pendientes → `Sender` → `sent`/`failed`.
/// Otros `kind` se dejan intactos (`skipped`): los atiende otro worker.
pub async fn drain_outbox_once<S: Sender>(
    pool: &PgPool,
    sender: &S,
    limit: i64,
) -> Result<DrainStats, AgentError> {
    let mut stats = DrainStats::default();
    for entry in fetch_pending_outbox(pool, limit).await? {
        if entry.kind != "send" {
            stats.skipped += 1;
            continue;
        }
        let Ok(out) = serde_json::from_value(entry.payload.clone()) else {
            mark_outbox(pool, entry.id, "failed").await?;
            stats.failed += 1;
            continue;
        };
        if sender.enviar(&out).await.is_ok() {
            mark_outbox(pool, entry.id, "sent").await?;
            stats.sent += 1;
        } else {
            mark_outbox(pool, entry.id, "failed").await?;
            stats.failed += 1;
        }
    }
    Ok(stats)
}
