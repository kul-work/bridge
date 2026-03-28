use crate::error::BridgeError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use uuid::Uuid;

/// API key for app authentication
/// Used by api_key middleware for authentication. Struct construction is handled by SQLx FromRow.
#[allow(dead_code)]
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub app_id: Uuid,
    pub key_prefix: String,
    pub key_hash: String,
    pub label: Option<String>,
    pub permissions: Vec<String>,
    pub enabled: bool,
}

pub async fn validate_api_key(pool: &PgPool, key_hash: &str) -> Result<Uuid, BridgeError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT app_id FROM pay.api_keys WHERE key_hash = $1 AND enabled = true"
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::UnauthorizedError("Invalid API key".to_string()))
}


