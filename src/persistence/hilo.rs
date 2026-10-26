//! Hilo completo para staff (F11) e import puntual de historial de canal.
//!
//! Paginación por `sequence_num` descendente: el cursor `antes` es la secuencia
//! más antigua ya vista. El import puntual inserta por delante del hilo con
//! secuencias negativas (`mínimo - 1`, ...): no choca con el asignador del hub
//! (que solo avanza desde el máximo) ni con la unicidad por sesión.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::eventos::registrar_evento;
use super::sesiones::{insertar_inbound, InboundCanal};
use crate::errors::AgentError;
use crate::models::ChatMessage;

/// Tope de mensajes por import puntual (plan F11: no es sync total).
pub const IMPORT_TOPE: usize = 100;
/// Página por defecto y tope del hilo: la consola no pide miles de golpe.
pub const HILO_DEFECTO: i64 = 50;
pub const HILO_TOPE: i64 = 100;
/// Remitentes válidos en un import: el historial de canal no dice si la IA respondió.
const SENDERS_IMPORT: [&str; 2] = ["client", "staff"];

#[derive(Debug, Serialize)]
pub struct PaginaHilo {
    /// Cronológico ascendente (lo más antiguo primero).
    pub mensajes: Vec<ChatMessage>,
    /// Cursor para pedir la página anterior; `None` cuando ya no hay más.
    pub siguiente_antes: Option<i64>,
}

/// Página del hilo anterior a `antes` (o la última si no hay cursor).
pub async fn listar_hilo(
    pool: &PgPool,
    session_id: Uuid,
    antes: Option<i64>,
    limite: i64,
) -> Result<PaginaHilo, AgentError> {
    let limite = limite.clamp(1, HILO_TOPE);
    let cursor = antes.unwrap_or(i64::MAX);
    // Pide una fila de más: sabe si queda historial sin una segunda consulta.
    let mut filas = sqlx::query_as::<_, ChatMessage>(
        "SELECT id, session_id, sender, body, sequence_num, input_tokens, output_tokens, created_at \
         FROM agent_messages WHERE session_id = $1 AND sequence_num < $2 \
         ORDER BY sequence_num DESC LIMIT $3",
    )
    .bind(session_id)
    .bind(cursor)
    .bind(limite + 1)
    .fetch_all(pool)
    .await?;
    let hay_mas = i64::try_from(filas.len()).unwrap_or(i64::MAX) > limite;
    filas.truncate(usize::try_from(limite).unwrap_or(0));
    // Descendente desde la BD: el último es el más antiguo de la página (cursor).
    let siguiente_antes = if hay_mas {
        filas.last().map(|m| m.sequence_num)
    } else {
        None
    };
    filas.reverse();
    Ok(PaginaHilo {
        mensajes: filas,
        siguiente_antes,
    })
}

/// Mensaje ya normalizado por el adaptador de canal (el fetch del historial vive allí).
#[derive(Debug, Clone, Deserialize)]
pub struct MensajeImport {
    /// `client` o `staff` (ver `SENDERS_IMPORT`).
    pub sender: String,
    pub body: String,
    /// Id del mensaje en el canal: es la clave de dedup `(canal, id_externo)`.
    pub id_externo: String,
}

#[derive(Debug, Serialize)]
pub struct ImportResumen {
    pub insertados: usize,
    /// Ya existían en el hilo (mismo `id_externo`): se descartan, no se duplican.
    pub duplicados: usize,
}

/// Secuencia del `indice`-ésimo mensaje importado (0 = el más reciente del lote).
/// Siempre por debajo de `base`, así que el historial importado queda antes del hilo vivo.
fn secuencia_importada(base: i64, indice: usize) -> i64 {
    // El lote tiene como mucho IMPORT_TOPE filas: la conversión nunca falla.
    base - 1 - i64::try_from(indice).unwrap_or(0)
}

/// Valida el lote antes de tocar la BD: canal, tamaño, remitente e id externo.
fn validar_lote(plantilla: &InboundCanal, mensajes: &[MensajeImport]) -> Result<(), AgentError> {
    if plantilla.canal.trim().is_empty() {
        return Err(AgentError::BadRequest("canal vacío".to_string()));
    }
    if mensajes.is_empty() || mensajes.len() > IMPORT_TOPE {
        return Err(AgentError::BadRequest(format!(
            "el import admite de 1 a {IMPORT_TOPE} mensajes"
        )));
    }
    for m in mensajes {
        if !SENDERS_IMPORT.contains(&m.sender.as_str()) {
            return Err(AgentError::BadRequest(format!(
                "remitente no admitido en import: {}",
                m.sender
            )));
        }
        // Sin id externo no hay dedup: reimportar duplicaría el hilo.
        if m.id_externo.trim().is_empty() {
            return Err(AgentError::BadRequest(
                "cada mensaje importado necesita id_externo".to_string(),
            ));
        }
    }
    Ok(())
}

/// Importa historial puntual de un canal: lote atómico, dedup por `id_externo`,
/// sin IA, outbox ni handoff. Queda auditado como evento `import.puntual`.
pub async fn importar_puntual(
    pool: &PgPool,
    session_id: Uuid,
    actor: &str,
    plantilla: &InboundCanal,
    mensajes: &[MensajeImport],
) -> Result<ImportResumen, AgentError> {
    validar_lote(plantilla, mensajes)?;
    let mut tx = pool.begin().await?;
    // Un lote a la vez por sesión: sin esto dos imports calcularían el mismo mínimo.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(session_id.to_string())
        .execute(&mut *tx)
        .await?;
    let base: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MIN(sequence_num), 1) FROM agent_messages WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_one(&mut *tx)
    .await?;
    let mut insertados = 0_usize;
    // Del más reciente al más antiguo: cada mensaje cae justo debajo del anterior.
    for (indice, m) in mensajes.iter().rev().enumerate() {
        let meta = InboundCanal {
            id_externo: Some(m.id_externo.clone()),
            imported: true,
            ..plantilla.clone()
        };
        let seq = secuencia_importada(base, indice);
        if insertar_inbound(&mut *tx, session_id, &m.sender, &m.body, seq, &meta)
            .await?
            .is_some()
        {
            insertados += 1;
        }
    }
    tx.commit().await?;
    let resumen = ImportResumen {
        insertados,
        duplicados: mensajes.len() - insertados,
    };
    let detalle = serde_json::json!({
        "canal": plantilla.canal,
        "insertados": resumen.insertados,
        "duplicados": resumen.duplicados,
    });
    registrar_evento(pool, session_id, "import.puntual", actor, &detalle).await?;
    Ok(resumen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canal() -> InboundCanal {
        InboundCanal {
            canal: "whatsapp".to_string(),
            ..InboundCanal::default()
        }
    }

    fn mensaje(sender: &str, id: &str) -> MensajeImport {
        MensajeImport {
            sender: sender.to_string(),
            body: "hola".to_string(),
            id_externo: id.to_string(),
        }
    }

    #[test]
    fn secuencia_importada_queda_antes_del_minimo() {
        assert_eq!(secuencia_importada(1, 0), 0);
        assert_eq!(secuencia_importada(1, 1), -1);
        assert_eq!(secuencia_importada(-5, 2), -8);
    }

    #[test]
    fn lote_valido_pasa() {
        let lote = vec![mensaje("client", "a1"), mensaje("staff", "a2")];
        assert!(validar_lote(&canal(), &lote).is_ok());
    }

    #[test]
    fn lote_vacio_o_sobre_tope_se_rechaza() {
        assert!(validar_lote(&canal(), &[]).is_err());
        let grande: Vec<MensajeImport> = (0..=IMPORT_TOPE)
            .map(|i| mensaje("client", &i.to_string()))
            .collect();
        assert!(validar_lote(&canal(), &grande).is_err());
    }

    #[test]
    fn remitente_ai_o_id_vacio_se_rechaza() {
        assert!(validar_lote(&canal(), &[mensaje("ai", "x")]).is_err());
        assert!(validar_lote(&canal(), &[mensaje("client", "  ")]).is_err());
    }

    #[test]
    fn canal_vacio_se_rechaza() {
        let sin_canal = InboundCanal::default();
        assert!(validar_lote(&sin_canal, &[mensaje("client", "x")]).is_err());
    }
}
