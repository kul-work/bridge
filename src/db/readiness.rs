use sqlx::PgPool;

use crate::error::BridgeError;

pub async fn count_enabled_provider_configs(pool: &PgPool) -> Result<i64, BridgeError> {
    sqlx::query_scalar("SELECT pay.count_enabled_provider_configs_bootstrap()")
        .fetch_one(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))
}
