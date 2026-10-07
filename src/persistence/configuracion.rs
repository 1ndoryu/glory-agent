//! Config del núcleo (`agent_config`): claves del producto sin tocar código.

use sqlx::PgPool;

use crate::errors::AgentError;
use crate::models::AgentConfig;

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
