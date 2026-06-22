use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::Html,
};
use uuid::Uuid;
use tracing::{info, error};

use crate::{
    config::ADMIN_WEBHOOK_LIST_LIMIT,
    db::apps as app_queries,
    error::BridgeError,
    ports::{AdminRepository, WebhookForwardRepository},
    state::AppState,
    utils::redact_with_prefix,
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
            notes: app.notes.clone(),
            failed_webhooks,
        });
    }

    Ok(axum::Json(summaries))
}

pub async fn update_app_notes(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Json(payload): Json<UpdateAppNotesRequest>,
) -> Result<Json<UpdateAppNotesResponse>, BridgeError> {
    let app_uuid = Uuid::parse_str(&app_id)
        .map_err(|_| BridgeError::ValidationError("Invalid app ID".to_string()))?;

    if payload.notes.len() > 4_000 {
        return Err(BridgeError::ValidationError(
            "Notes cannot exceed 4000 characters".to_string(),
        ));
    }

    let database = state.database();
    let notes = app_queries::update_app_notes(database.pool(), app_uuid, &payload.notes).await?;

    Ok(Json(UpdateAppNotesResponse { id: app_id, notes }))
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

pub async fn get_webhook_payload(
    State(state): State<AppState>,
    Path(webhook_id): Path<String>,
) -> Result<axum::Json<WebhookPayloadSummary>, BridgeError> {
    let webhook_uuid = Uuid::parse_str(&webhook_id)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook ID".to_string()))?;

    let database = state.database();
    let provider = database
        .as_ref()
        .get_webhook_provider_for_delivery(webhook_uuid)
        .await?;

    Ok(axum::Json(WebhookPayloadSummary {
        id: webhook_id,
        provider: provider.provider,
        provider_webhook_id: provider.provider_webhook_id,
        event_type: provider.event_type,
        redacted_payload: redact_payload(provider.payload),
    }))
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

/// Manually trigger background jobs on demand.
/// Works even when ENABLE_BACKGROUND_JOBS=false.
/// Jobs run inline (not spawned) so the response reflects actual success or failure.
pub async fn trigger_jobs(
    State(state): State<AppState>,
    Json(payload): Json<TriggerJobsRequest>,
) -> Result<Json<TriggerJobsResponse>, BridgeError> {
    const VALID_JOBS: &[&str] = &[
        "webhook_retry",
        "reconciliation",
        "price_step_up",
        "pause_scheduler",
        "cleanup",
    ];

    let jobs: Vec<String> = if payload.jobs.iter().any(|j| j == "all") {
        VALID_JOBS.iter().map(|s| s.to_string()).collect()
    } else {
        payload.jobs
    };

    if jobs.is_empty() {
        return Err(BridgeError::ValidationError(
            "No jobs specified. Valid jobs: webhook_retry, reconciliation, price_step_up, pause_scheduler, cleanup, all".to_string(),
        ));
    }

    for job in &jobs {
        if job != "all" && !VALID_JOBS.contains(&job.as_str()) {
            return Err(BridgeError::ValidationError(format!(
                "Unknown job: '{}'. Valid jobs: webhook_retry, reconciliation, price_step_up, pause_scheduler, cleanup, all",
                job
            )));
        }
    }

    let db = state.database();
    let mut results = Vec::new();

    for job in jobs {
        info!("Manual trigger: {} job", job);
        let result = match job.as_str() {
            "webhook_retry" => {
                let mut err = None;
                if let Err(e) = crate::webhooks::scheduler::retry_webhooks(db.as_ref()).await {
                    error!("Manual webhook retry job failed: {}", e);
                    err = Some(e.to_string());
                }
                if let Err(e) = crate::webhooks::scheduler::retry_google_play_subscription_acknowledgements(db.as_ref()).await {
                    error!("Manual Google Play acknowledgement retry failed: {}", e);
                    err = Some(e.to_string());
                }
                err
            }
            "reconciliation" => {
                match crate::webhooks::scheduler::reconcile_subscriptions(&db).await {
                    Ok(()) => None,
                    Err(e) => {
                        error!("Manual reconciliation job failed: {}", e);
                        Some(e.to_string())
                    }
                }
            }
            "price_step_up" => {
                match crate::webhooks::scheduler::process_price_step_up_expiry(&db).await {
                    Ok(()) => None,
                    Err(e) => {
                        error!("Manual price step-up expiry job failed: {}", e);
                        Some(e.to_string())
                    }
                }
            }
            "pause_scheduler" => {
                match crate::webhooks::scheduler::process_pause_transitions(&db).await {
                    Ok(()) => None,
                    Err(e) => {
                        error!("Manual pause scheduler job failed: {}", e);
                        Some(e.to_string())
                    }
                }
            }
            "cleanup" => {
                match crate::webhooks::scheduler::cleanup_old_data(&db).await {
                    Ok(()) => None,
                    Err(e) => {
                        error!("Manual cleanup job failed: {}", e);
                        Some(e.to_string())
                    }
                }
            }
            _ => unreachable!("validated above"),
        };
        results.push(JobResult { job, error: result });
    }

    info!("Admin trigger-jobs completed: {:?}", results.iter().map(|r| &r.job).collect::<Vec<_>>());

    Ok(Json(TriggerJobsResponse { results }))
}

#[derive(serde::Deserialize)]
pub struct TriggerJobsRequest {
    #[serde(default)]
    pub jobs: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct TriggerJobsResponse {
    pub results: Vec<JobResult>,
}

#[derive(serde::Serialize)]
pub struct JobResult {
    pub job: String,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AppSummary {
    pub id: String,
    pub slug: String,
    pub display_name: String,
    pub app_url: Option<String>,
    pub notes: Option<String>,
    pub failed_webhooks: i64,
}

#[derive(serde::Deserialize)]
pub struct UpdateAppNotesRequest {
    pub notes: String,
}

#[derive(serde::Serialize)]
pub struct UpdateAppNotesResponse {
    pub id: String,
    pub notes: Option<String>,
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

#[derive(serde::Serialize)]
pub struct WebhookPayloadSummary {
    pub id: String,
    pub provider: String,
    pub provider_webhook_id: String,
    pub event_type: String,
    pub redacted_payload: serde_json::Value,
}

fn redact_payload(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    if should_redact_payload_key(&key) {
                        (key, redact_payload_value(value))
                    } else {
                        (key, redact_payload(value))
                    }
                })
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(redact_payload).collect())
        }
        other => other,
    }
}

fn redact_payload_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(value) => serde_json::Value::String(redact_with_prefix(&value)),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::Value::String("[redacted]".to_string())
        }
        serde_json::Value::Null => serde_json::Value::Null,
        _ => serde_json::Value::String("[redacted]".to_string()),
    }
}

fn should_redact_payload_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let contains_sensitive = [
        "authorization",
        "checkout_id",
        "customer_email",
        "email",
        "password",
        "purchase_token",
        "purchasetoken",
        "secret",
        "token",
    ]
    .iter()
    .any(|sensitive| key.contains(sensitive));
    let exact_sensitive = [
        "customer_name",
        "first_name",
        "firstname",
        "last_name",
        "lastname",
        "name",
    ]
    .iter()
    .any(|sensitive| key == *sensitive);

    contains_sensitive || exact_sensitive
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::redact_payload;

    #[test]
    fn redact_payload_masks_sensitive_keys_recursively() {
        let redacted = redact_payload(json!({
            "id": "evt_123",
            "customer_email": "admin@example.com",
            "object": {
                "purchaseToken": "long_purchase_token_1234567890",
                "amount": 499,
                "packageName": "com.tyde.bridge"
            }
        }));

        assert_eq!(redacted["id"], "evt_123");
        assert_eq!(redacted["customer_email"], "[redacted]...mple.com");
        assert_eq!(redacted["object"]["purchaseToken"], "[redacted]...34567890");
        assert_eq!(redacted["object"]["amount"], 499);
        assert_eq!(redacted["object"]["packageName"], "com.tyde.bridge");
    }
}
