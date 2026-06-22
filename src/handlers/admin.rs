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
    ports::{AdminRepository, WebhookForwardRepository},
    state::AppState,
};

/// Get admin dashboard page.
/// Serves the HTML with the Clerk publishable key injected for client-side auth.
pub async fn admin_dashboard(
    State(_state): State<AppState>,
) -> Result<Html<String>, BridgeError> {
    let publishable_key = std::env::var("CLERK_PUBLISHABLE_KEY")
        .map_err(|_| BridgeError::ConfigError(
            "CLERK_PUBLISHABLE_KEY must be set for admin auth.".to_string()
        ))?;

    let html = include_str!("../../templates/admin.html");
    let html = html.replace("{{CLERK_PUBLISHABLE_KEY}}", &publishable_key);
    Ok(Html(html))
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

/// Retry webhook delivery manually.
/// Resets dead-lettered/pending deliveries so the background retry worker
/// picks them up on its next tick. Does not forward directly to avoid racing
/// the worker and to ensure suppressed webhooks are handled correctly.
pub async fn retry_webhook(
    State(state): State<AppState>,
    Path(webhook_id): Path<String>,
) -> Result<StatusCode, BridgeError> {
    let webhook_uuid = Uuid::parse_str(&webhook_id)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook ID".to_string()))?;

    let database = state.database();

    let delivery = database.as_ref().get_webhook_delivery(webhook_uuid).await?;

    if delivery.forwarded {
        info!("Skipping manual retry for already-forwarded webhook {}", webhook_uuid);
        return Ok(StatusCode::OK);
    }

    info!(
        "Manual retry queued for webhook delivery {} (app={}, attempts={}, dead_lettered={})",
        webhook_uuid, delivery.app_id, delivery.forward_attempts, delivery.dead_lettered
    );

    database.as_ref().reset_webhook_delivery(webhook_uuid).await?;

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
