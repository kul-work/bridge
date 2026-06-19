use crate::error::BridgeError;
use crate::db::database::set_local_app_id;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct App {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub google_package_name: Option<String>,
    pub apple_bundle_id: Option<String>,
    pub webhook_callback_url: String,
    pub webhook_callback_secret: String,
    pub webhook_ingress_token: Uuid,
    pub api_rate_limit_per_minute: i32,
    pub api_rate_limit_rules: Option<serde_json::Value>,
    pub app_url: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AppSummary {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub app_url: Option<String>,
}

pub async fn get_app(pool: &PgPool, app_id: Uuid) -> Result<App, BridgeError> {
    let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;

    let app = sqlx::query_as::<_, App>(
        "SELECT * FROM pay.apps WHERE id = $1"
    )
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ValidationError("App not found".to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(app)
}

pub async fn list_enabled_app_ids(pool: &PgPool) -> Result<Vec<Uuid>, BridgeError> {
    sqlx::query_scalar("SELECT id FROM pay.list_enabled_app_ids_bootstrap()")
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn list_app_summaries(pool: &PgPool) -> Result<Vec<AppSummary>, BridgeError> {
    sqlx::query_as::<_, AppSummary>(
        "SELECT id, slug, display_name, app_url FROM pay.list_app_summaries_bootstrap()"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

/// Get app by webhook ingress token
pub async fn get_app_by_webhook_token(pool: &PgPool, token: Uuid) -> Result<App, BridgeError> {
    sqlx::query_as::<_, App>(
        "SELECT * FROM pay.get_app_by_webhook_token_bootstrap($1)"
    )
    .bind(token)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ValidationError("App not found".to_string()))
}