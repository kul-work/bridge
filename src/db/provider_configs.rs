use crate::error::BridgeError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: Uuid,
    pub app_id: Uuid,
    pub provider: String,
    pub config: serde_json::Value,
    pub enabled: bool,
}

pub async fn get_provider_config(
    pool: &PgPool,
    app_id: Uuid,
    provider: &str,
) -> Result<ProviderConfig, BridgeError> {
    sqlx::query_as::<_, ProviderConfig>(
        "SELECT * FROM pay.provider_configs 
         WHERE app_id = $1 AND provider = $2 AND enabled = true"
    )
    .bind(app_id)
    .bind(provider)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ConfigError(format!("Provider {} not configured", provider)))
}
