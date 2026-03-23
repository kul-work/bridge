use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Html,
};
use std::sync::Arc;
use uuid::Uuid;
use tracing::info;

use crate::{db::Database, error::BridgeError};

/// Get admin dashboard page
pub async fn admin_dashboard(
    State(_db): State<Arc<Database>>,
) -> Result<Html<String>, BridgeError> {
    // For now, return a simple placeholder
    let html = include_str!("../../templates/admin.html");
    Ok(Html(html.to_string()))
}

/// Get list of apps (JSON)
pub async fn list_apps(
    State(db): State<Arc<Database>>,
) -> Result<axum::Json<Vec<AppSummary>>, BridgeError> {
    // Query apps from database
    let apps = sqlx::query_as::<_, crate::db::apps::App>(
        "SELECT * FROM pay.apps ORDER BY display_name"
    )
    .fetch_all(&db.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let mut summaries = Vec::new();
    for app in apps {
        let failed_webhooks = crate::db::webhooks::count_failed_webhooks(&db.pool, app.id)
            .await
            .unwrap_or(0);

        summaries.push(AppSummary {
            id: app.id.to_string(),
            slug: app.slug.clone(),
            display_name: app.display_name.clone(),
            app_url: app.app_url.clone(),
            failed_webhooks,
        });
    }

    Ok(axum::Json(summaries))
}

/// Get webhooks for an app
pub async fn get_app_webhooks(
    State(db): State<Arc<Database>>,
    Path(app_id): Path<String>,
) -> Result<axum::Json<Vec<WebhookSummary>>, BridgeError> {
    let app_uuid = Uuid::parse_str(&app_id)
        .map_err(|_| BridgeError::ValidationError("Invalid app ID".to_string()))?;

    let webhooks = crate::db::webhooks::list_app_webhooks(&db.pool, app_uuid, 50, 0)
        .await?;

    let summaries = webhooks
        .iter()
        .map(|(delivery, provider)| WebhookSummary {
            id: delivery.id.to_string(),
            provider_webhook_id: provider.provider_webhook_id.clone(),
            event_type: provider.event_type.clone(),
            provider: provider.provider.clone(),
            forwarded: delivery.forwarded,
            forward_attempts: delivery.forward_attempts,
            last_http_status: delivery.last_http_status,
            last_error: delivery.last_error.clone(),
            created_at: delivery.created_at.to_rfc3339(),
        })
        .collect();

    Ok(axum::Json(summaries))
}

/// Retry webhook delivery manually
pub async fn retry_webhook(
    State(_db): State<Arc<Database>>,
    Path(webhook_id): Path<String>,
) -> Result<StatusCode, BridgeError> {
    let webhook_uuid = Uuid::parse_str(&webhook_id)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook ID".to_string()))?;

    info!("Manual retry requested for webhook: {}", webhook_uuid);

    // TODO: Queue the webhook for retry

    Ok(StatusCode::OK)
}

#[derive(serde::Serialize)]
pub struct AppSummary {
    pub id: String,
    pub slug: String,
    pub display_name: String,
    pub app_url: Option<String>,
    pub failed_webhooks: i64,
}

#[derive(serde::Serialize)]
pub struct WebhookSummary {
    pub id: String,
    pub provider_webhook_id: String,
    pub event_type: String,
    pub provider: String,
    pub forwarded: bool,
    pub forward_attempts: i32,
    pub last_http_status: Option<i32>,
    pub last_error: Option<String>,
    pub created_at: String,
}
