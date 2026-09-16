/* Errores del crate: explícitos, sin fallos silenciosos. */

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("ai error: {0}")]
    Ai(String),
    #[error("db error: {0}")]
    Db(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for AgentError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<reqwest::Error> for AgentError {
    fn from(e: reqwest::Error) -> Self {
        Self::Ai(e.to_string())
    }
}

impl IntoResponse for AgentError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            Self::RateLimited(m) => (StatusCode::TOO_MANY_REQUESTS, m.clone()),
            Self::Ai(m) => (StatusCode::BAD_GATEWAY, m.clone()),
            Self::Db(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.clone()),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.clone()),
        };
        tracing::error!("glory-agent error: {self}");
        (status, Json(json!({ "ok": false, "error": msg }))).into_response()
    }
}
