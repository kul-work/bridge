use crate::error::BridgeError;
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

pub async fn get_app(pool: &PgPool, app_id: Uuid) -> Result<App, BridgeError> {
    sqlx::query_as::<_, App>(
        "SELECT * FROM pay.apps WHERE id = $1"
    )
    .bind(app_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ValidationError("App not found".to_string()))
}

pub async fn list_enabled_apps(pool: &PgPool) -> Result<Vec<App>, BridgeError> {
    sqlx::query_as::<_, App>(
        "SELECT * FROM pay.apps WHERE enabled = true"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn list_apps(pool: &PgPool) -> Result<Vec<App>, BridgeError> {
    sqlx::query_as::<_, App>(
        "SELECT * FROM pay.apps ORDER BY display_name"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

/// Get app by webhook ingress token
/// TODO: Used by webhook ingress handlers (not yet implemented) to map incoming webhooks to apps.
pub async fn get_app_by_webhook_token(pool: &PgPool, token: Uuid) -> Result<App, BridgeError> {
    sqlx::query_as::<_, App>(
        "SELECT * FROM pay.apps WHERE webhook_ingress_token = $1 AND enabled = true"
    )
    .bind(token)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ValidationError("App not found".to_string()))
}
