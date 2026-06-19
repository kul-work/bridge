use crate::db::database::set_local_app_id;
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
    let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;

    let config = sqlx::query_as::<_, ProviderConfig>(
        "SELECT * FROM pay.provider_configs 
         WHERE app_id = $1 AND provider = $2 AND enabled = true"
    )
    .bind(app_id)
    .bind(provider)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ProviderNotConfigured(format!("Provider {} not configured", provider)))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(config)
}
