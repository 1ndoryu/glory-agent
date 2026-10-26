/* sentinel-disable-file ruta-post-sin-rate-limit: el límite de peticiones es de la app que monta el router (edge/gateway); esta lib no posee el servidor HTTP ni añade tower_governor. Las rutas POST exigen la auth de la app. */
/* Consola (F10/F4): endpoints admin para que una app opere el hilo sin tocar SQL.
 * Sin auth aquí: la lib no guarda secretos. La app monta `admin_routes()` detrás de
 * su propia capa de autenticación; nunca debe quedar abierta. */

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    errors::AgentError,
    models::{AgentConfig, AgentEvento, ChatSession, ResumenUso},
    persistence,
    transport::AgentState,
};

/// Eventos por defecto en `GET /agent/admin/eventos` y tope duro (la consola no pide miles).
const EVENTOS_DEFECTO: i64 = 50;
const EVENTOS_TOPE: i64 = 200;

#[derive(Debug, Deserialize)]
pub struct HandoffBody {
    pub session_id: Uuid,
    /// `tomar|devolver|cerrar|reabrir`; la validación real está en `handoff::AccionHandoff`.
    pub accion: String,
    /// Staff genérico (nunca datos del visitante): va a la auditoría.
    pub actor: String,
    pub motivo: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EventosParams {
    pub session_id: Uuid,
    pub limite: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UsoParams {
    pub session_id: Uuid,
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ValorBody {
    pub valor: String,
}

/// Rutas de consola. La app las monta detrás de su auth; ver cabecera del módulo.
pub fn admin_routes() -> Router<AgentState> {
    Router::new()
        .route("/agent/admin/handoff", post(handoff))
        .route("/agent/admin/eventos", get(eventos))
        .route("/agent/admin/uso", get(uso))
        .route("/agent/admin/config", get(config_listar))
        .route("/agent/admin/config/:clave", put(config_fijar))
}

/// Sin pool no hay consola: error explícito, nunca una respuesta vacía que parezca válida.
fn pool_de(state: &AgentState) -> Result<&sqlx::PgPool, AgentError> {
    state
        .pool
        .as_ref()
        .ok_or_else(|| AgentError::Internal("consola sin base de datos".to_string()))
}

/// Acota el límite pedido al rango admitido; sin valor, usa el defecto.
fn acotar_limite(pedido: Option<i64>) -> i64 {
    pedido.unwrap_or(EVENTOS_DEFECTO).clamp(1, EVENTOS_TOPE)
}

async fn handoff(
    State(state): State<AgentState>,
    Json(body): Json<HandoffBody>,
) -> Result<Json<ChatSession>, AgentError> {
    let pool = pool_de(&state)?;
    let sesion = persistence::handoff(
        pool,
        body.session_id,
        &body.accion,
        &body.actor,
        body.motivo.as_deref(),
    )
    .await?;
    Ok(Json(sesion))
}

async fn eventos(
    State(state): State<AgentState>,
    Query(params): Query<EventosParams>,
) -> Result<Json<Vec<AgentEvento>>, AgentError> {
    let pool = pool_de(&state)?;
    let filas =
        persistence::listar_eventos(pool, params.session_id, acotar_limite(params.limite)).await?;
    Ok(Json(filas))
}

async fn uso(
    State(state): State<AgentState>,
    Query(params): Query<UsoParams>,
) -> Result<Json<ResumenUso>, AgentError> {
    let pool = pool_de(&state)?;
    let resumen =
        persistence::resumen_uso(pool, params.session_id, params.desde, params.hasta).await?;
    Ok(Json(resumen))
}

async fn config_listar(
    State(state): State<AgentState>,
) -> Result<Json<Vec<AgentConfig>>, AgentError> {
    let pool = pool_de(&state)?;
    Ok(Json(persistence::list_config(pool).await?))
}

async fn config_fijar(
    State(state): State<AgentState>,
    Path(clave): Path<String>,
    Json(body): Json<ValorBody>,
) -> Result<Json<AgentConfig>, AgentError> {
    let pool = pool_de(&state)?;
    Ok(Json(
        persistence::set_config(pool, &clave, &body.valor).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limite_de_eventos_se_acota() {
        assert_eq!(acotar_limite(None), EVENTOS_DEFECTO);
        assert_eq!(acotar_limite(Some(0)), 1);
        assert_eq!(acotar_limite(Some(9_999)), EVENTOS_TOPE);
        assert_eq!(acotar_limite(Some(20)), 20);
    }

    #[test]
    fn handoff_body_acepta_motivo_opcional() {
        let sin: HandoffBody = serde_json::from_str(
            r#"{"session_id":"00000000-0000-0000-0000-000000000001","accion":"tomar","actor":"ana"}"#,
        )
        .expect("parse");
        assert!(sin.motivo.is_none());
    }
}
