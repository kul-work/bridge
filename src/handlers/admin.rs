use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Html,
};
use uuid::Uuid;
use tracing::info;

use crate::{
    config::ADMIN_WEBHOOK_LIST_LIMIT,
    error::BridgeError,
    ports::AdminRepository,
    state::AppState,
};

/// Get admin dashboard page
pub async fn admin_dashboard() -> Result<Html<String>, BridgeError> {
    // For now, return a simple placeholder
    let html = include_str!("../../templates/admin.html");
    Ok(Html(html.to_string()))
}

/// Get list of apps (JSON)
pub async fn list_apps(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<AppSummary>>, BridgeError> {
    let database = state.database();
    let apps = database.as_ref().list_app_summaries().await?;

    let mut summaries = Vec::new();
    for app in apps {
        let failed_webhooks = database
            .as_ref()
            .count_failed_webhooks(app.id)
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
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Result<axum::Json<Vec<WebhookSummary>>, BridgeError> {
    let database = state.database();
    let app_uuid = Uuid::parse_str(&app_id)
        .map_err(|_| BridgeError::ValidationError("Invalid app ID".to_string()))?;

    let webhooks = database
        .as_ref()
        .list_app_webhooks(app_uuid, ADMIN_WEBHOOK_LIST_LIMIT, 0)
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
            dead_lettered: delivery.dead_lettered,
            dead_lettered_at: delivery.dead_lettered_at.map(|value| value.to_rfc3339()),
            dead_letter_reason: delivery.dead_letter_reason.clone(),
            last_http_status: delivery.last_http_status,
            last_error: delivery.last_error.clone(),
            created_at: delivery.created_at.to_rfc3339(),
        })
        .collect();

    Ok(axum::Json(summaries))
}

/// Retry webhook delivery manually
pub async fn retry_webhook(
    Path(webhook_id): Path<String>,
) -> Result<StatusCode, BridgeError> {
    let webhook_uuid = Uuid::parse_str(&webhook_id)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook ID".to_string()))?;

    info!("Manual retry requested for webhook: {}", webhook_uuid);

    // TODO: Queue the webhook for retry
    //Smallest sane fix: make the admin handler accept State<AppState>, load the delivery, rebuild its canonical payload via build_canonical_payload, and call
    //forward_webhook immediately. That matches existing retry behavior without inventing a new queue. If manual retry is meant to resurrect dead-lettered
    //deliveries, it also needs an explicit DB helper to clear dead_lettered / reset attempts; current forwarding code refuses dead-lettered or >= 3 rows.

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
    pub dead_lettered: bool,
    pub dead_lettered_at: Option<String>,
    pub dead_letter_reason: Option<String>,
    pub last_http_status: Option<i32>,
    pub last_error: Option<String>,
    pub created_at: String,
}
