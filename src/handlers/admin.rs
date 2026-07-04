use axum::{
    extract::{Extension, Json, Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode},
    middleware::Next,
    response::Response,
    response::{Html, IntoResponse},
};
use std::{collections::HashSet, sync::OnceLock};
use tokio::sync::Mutex;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    config::ADMIN_WEBHOOK_LIST_LIMIT,
    db::apps as app_queries,
    error::BridgeError,
    middleware::admin_auth::AdminAuthContext,
    ports::{AdminRepository, WebhookForwardRepository},
    state::AppState,
    utils::redact_with_prefix,
};

static ADMIN_OPERATION_LOCKS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub async fn admin_no_store_middleware(request: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        HeaderName::from_static("cache-control"),
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("pragma"),
        HeaderValue::from_static("no-cache"),
    );
    response
}

/// Get admin dashboard page.
/// Serves the HTML with the Clerk publishable key injected for client-side auth.
pub async fn admin_dashboard(
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, BridgeError> {
    let publishable_key = std::env::var("CLERK_PUBLISHABLE_KEY").map_err(|_| {
        BridgeError::ConfigError("CLERK_PUBLISHABLE_KEY must be set for admin auth.".to_string())
    })?;
    let csp_nonce = Uuid::new_v4().simple().to_string();

    let environment = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string());
    let bypass_enabled = crate::config::parse_bool_env("BYPASS_ADMIN_AUTH", false).unwrap_or(false);
    let bypass_allowed = crate::middleware::admin_auth::admin_auth_bypass_allowed(&environment, bypass_enabled);

    let allow_emergency = crate::config::parse_bool_env("ALLOW_EMERGENCY_CLEANUP", false).unwrap_or(false);
    let show_emergency = if allow_emergency { "block" } else { "none" };

    let html = include_str!("../../templates/admin.html");
    let html = html
        .replace("{{CLERK_PUBLISHABLE_KEY}}", &publishable_key)
        .replace("{{CSP_NONCE}}", &csp_nonce)
        .replace("{{BYPASS_ADMIN_AUTH}}", &bypass_allowed.to_string())
        .replace("{{SHOW_EMERGENCY_CLEANUP}}", show_emergency);
    Ok((admin_security_headers(&csp_nonce), Html(html)))
}

fn admin_security_headers(csp_nonce: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let csp = format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{0}' https://cdn.jsdelivr.net https://*.clerk.accounts.dev https://*.clerk.com https://hcaptcha.com https://*.hcaptcha.com https://challenges.cloudflare.com; \
         style-src 'self' 'unsafe-inline' https://hcaptcha.com https://*.hcaptcha.com; \
         connect-src 'self' https://*.clerk.accounts.dev https://*.clerk.com https://api.clerk.com https://hcaptcha.com https://*.hcaptcha.com https://challenges.cloudflare.com; \
         frame-src https://*.clerk.accounts.dev https://*.clerk.com https://hcaptcha.com https://*.hcaptcha.com https://challenges.cloudflare.com; \
         img-src 'self' data: blob: https://img.clerk.com https://images.clerk.dev https://*.clerk.com https://*.clerk.accounts.dev https://hcaptcha.com https://*.hcaptcha.com https://challenges.cloudflare.com; \
         font-src 'self' data: https://*.clerk.com https://*.clerk.accounts.dev https://*.perplexity.ai; \
         worker-src 'self' blob:; \
         object-src 'none'; \
         base-uri 'none'; \
         frame-ancestors 'none'",
        csp_nonce,
    );

    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_str(&csp).expect("Failed to build CSP header value"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    headers
}

async fn try_start_admin_operation(key: &str) -> bool {
    let locks = ADMIN_OPERATION_LOCKS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut locks = locks.lock().await;
    locks.insert(key.to_string())
}

async fn finish_admin_operation(key: &str) {
    if let Some(locks) = ADMIN_OPERATION_LOCKS.get() {
        let mut locks = locks.lock().await;
        locks.remove(key);
    }
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

pub async fn alert_dashboard(
    State(state): State<AppState>,
) -> Result<Json<Vec<AlertSignalSummary>>, BridgeError> {
    let database = state.database();
    let apps = database.as_ref().list_app_summaries().await?;
    let mut dead_lettered = 0;
    let mut retryable_failed = 0;
    let mut reconciliation_drift = 0;

    for app in apps {
        dead_lettered += database
            .as_ref()
            .count_dead_lettered_webhooks(app.id)
            .await?;
        retryable_failed += database
            .as_ref()
            .count_retryable_failed_webhooks(app.id)
            .await?;
        reconciliation_drift += database
            .as_ref()
            .count_reconciliation_drift_callbacks(app.id)
            .await?;
    }

    Ok(Json(vec![
        AlertSignalSummary::counted(
            "bridge.webhook.dead_lettered",
            "alert_signal",
            "ticket",
            "Webhook delivery dead-lettered",
            dead_lettered,
            "DB current: dead-lettered webhook_delivery rows",
        ),
        AlertSignalSummary::counted(
            "bridge.callback.delivery_failed",
            "support_debug_signal",
            "dashboard",
            "Webhook delivery retryable failure",
            retryable_failed,
            "DB current: failed deliveries still eligible for retry",
        ),
        AlertSignalSummary::counted(
            "bridge.reconciliation.drift_detected",
            "alert_signal",
            "audit",
            "Provider reconciliation drift detected",
            reconciliation_drift,
            "DB total: reconciliation drift callback records",
        ),
        AlertSignalSummary::log_routed(
            "bridge.reconciliation.job_failed",
            "alert_signal",
            "ticket",
            "Reconciliation job failed",
        ),
        AlertSignalSummary::log_routed(
            "bridge.email.auth_or_permission_failed",
            "alert_signal",
            "ticket",
            "Email provider auth or permission failure",
        ),
        AlertSignalSummary::log_routed(
            "bridge.db.readiness_failed",
            "alert_signal",
            "page",
            "Bridge database readiness failed",
        ),
        AlertSignalSummary::log_routed(
            "bridge.db.role_or_rls_failed",
            "alert_signal",
            "page",
            "Bridge database role or RLS failed",
        ),
    ]))
}

pub async fn update_app_notes(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Extension(admin): Extension<AdminAuthContext>,
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
    let notes = match app_queries::update_app_notes(database.pool(), app_uuid, &payload.notes).await
    {
        Ok(notes) => notes,
        Err(e) => {
            error!(
                admin_subject = %admin.subject,
                admin_org = ?admin.org_id,
                action = "update_app_notes",
                target_app_id = %app_uuid,
                result = "error",
                error = %e,
                "Admin mutation failed"
            );
            return Err(e);
        }
    };

    info!(
        admin_subject = %admin.subject,
        admin_org = ?admin.org_id,
        action = "update_app_notes",
        target_app_id = %app_uuid,
        result = "success",
        "Admin mutation completed"
    );

    Ok(Json(UpdateAppNotesResponse { id: app_id, notes }))
}

#[derive(serde::Deserialize)]
pub struct WebhooksQuery {
    pub page: Option<i64>,
}

#[derive(serde::Serialize)]
pub struct PaginatedWebhooks {
    pub webhooks: Vec<WebhookSummary>,
    pub total_count: i64,
    pub page: i64,
    pub pages: i64,
}

/// Get webhooks for an app
pub async fn get_app_webhooks(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(query): Query<WebhooksQuery>,
) -> Result<axum::Json<PaginatedWebhooks>, BridgeError> {
    let database = state.database();
    let app_uuid = Uuid::parse_str(&app_id)
        .map_err(|_| BridgeError::ValidationError("Invalid app ID".to_string()))?;

    let page = query.page.unwrap_or(1).max(1);
    let limit = ADMIN_WEBHOOK_LIST_LIMIT;
    let offset = (page - 1) * limit;

    let total_count = database.as_ref().count_app_webhooks(app_uuid).await?;

    let webhooks = database
        .as_ref()
        .list_app_webhooks(app_uuid, limit, offset)
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

    let pages = if total_count == 0 {
        1
    } else {
        (total_count + limit - 1) / limit
    };

    Ok(axum::Json(PaginatedWebhooks {
        webhooks: summaries,
        total_count,
        page,
        pages,
    }))
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
/// Resets dead-lettered deliveries so the background retry worker
/// picks them up on its next tick. Does not forward directly to avoid racing
/// the worker and to ensure suppressed webhooks are handled correctly.
pub async fn retry_webhook(
    State(state): State<AppState>,
    Path(webhook_id): Path<String>,
    Extension(admin): Extension<AdminAuthContext>,
) -> Result<StatusCode, BridgeError> {
    let webhook_uuid = Uuid::parse_str(&webhook_id)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook ID".to_string()))?;

    let lock_key = format!("retry_webhook:{}", webhook_uuid);
    if !try_start_admin_operation(&lock_key).await {
        info!(
            admin_subject = %admin.subject,
            admin_org = ?admin.org_id,
            action = "retry_webhook",
            target_webhook_id = %webhook_uuid,
            result = "already_running",
            "Admin mutation skipped"
        );
        return Err(BridgeError::Conflict(
            "A retry for this webhook delivery is already running".to_string(),
        ));
    }

    let database = state.database();

    let queued = match database.as_ref().reset_webhook_delivery(webhook_uuid).await {
        Ok(queued) => queued,
        Err(e) => {
            finish_admin_operation(&lock_key).await;
            error!(
                admin_subject = %admin.subject,
                admin_org = ?admin.org_id,
                action = "retry_webhook",
                target_webhook_id = %webhook_uuid,
                result = "error",
                error = %e,
                "Admin mutation failed"
            );
            return Err(e);
        }
    };

    finish_admin_operation(&lock_key).await;

    if queued {
        info!(
            admin_subject = %admin.subject,
            admin_org = ?admin.org_id,
            action = "retry_webhook",
            target_webhook_id = %webhook_uuid,
            result = "queued",
            "Admin mutation completed"
        );
    } else {
        info!(
            admin_subject = %admin.subject,
            admin_org = ?admin.org_id,
            action = "retry_webhook",
            target_webhook_id = %webhook_uuid,
            result = "not_dead_lettered",
            "Admin mutation skipped"
        );
    }

    Ok(StatusCode::OK)
}

/// Manually trigger background jobs on demand.
/// Works even when ENABLE_BACKGROUND_JOBS=false.
/// Jobs run inline (not spawned) so the response reflects actual success or failure.
pub async fn trigger_jobs(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminAuthContext>,
    Json(payload): Json<TriggerJobsRequest>,
) -> Result<Json<TriggerJobsResponse>, BridgeError> {
    const VALID_JOBS: &[&str] = &[
        "webhook_retry",
        "reconciliation",
        "price_step_up",
        "pause_scheduler",
        "cleanup",
        "webhook_retry_cleanup",
        "reconciliation_cleanup",
        "price_step_up_cleanup",
        "pause_scheduler_cleanup",
        "reset_stuck_workers",
    ];

    let jobs: Vec<String> = if payload.jobs.iter().any(|j| j == "all") {
        VALID_JOBS.iter().map(|s| s.to_string()).collect()
    } else {
        payload.jobs
    };

    if jobs.is_empty() {
        return Err(BridgeError::ValidationError(
            "No jobs specified. Valid jobs: webhook_retry, reconciliation, price_step_up, pause_scheduler, cleanup, webhook_retry_cleanup, reconciliation_cleanup, price_step_up_cleanup, pause_scheduler_cleanup, reset_stuck_workers, all".to_string(),
        ));
    }

    for job in &jobs {
        if job != "all" && !VALID_JOBS.contains(&job.as_str()) {
            return Err(BridgeError::ValidationError(format!(
                "Unknown job: '{}'. Valid jobs: webhook_retry, reconciliation, price_step_up, pause_scheduler, cleanup, webhook_retry_cleanup, reconciliation_cleanup, price_step_up_cleanup, pause_scheduler_cleanup, reset_stuck_workers, all",
                job
            )));
        }
    }

    let environment = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string());
    let is_prod = crate::config::is_production_environment(&environment);
    let allow_emergency = crate::config::parse_bool_env("ALLOW_EMERGENCY_CLEANUP", false).unwrap_or(false);

    for job in &jobs {
        if is_prod && job.ends_with("_cleanup") && !allow_emergency {
            return Err(BridgeError::ValidationError(format!(
                "Emergency cleanup job '{}' is disabled in production. Set ALLOW_EMERGENCY_CLEANUP=true in env to override this.",
                job
            )));
        }
    }

    let mut seen_jobs = HashSet::new();
    for job in &jobs {
        if !seen_jobs.insert(job.as_str()) {
            return Err(BridgeError::ValidationError(format!(
                "Duplicate job: '{}'. Submit each job only once",
                job
            )));
        }
    }

    let db = state.database();
    let mut results = Vec::new();

    for job in jobs {
        let lock_key = format!("trigger_job:{}", job);
        if !try_start_admin_operation(&lock_key).await {
            info!(
                admin_subject = %admin.subject,
                admin_org = ?admin.org_id,
                action = "trigger_job",
                target_job = %job,
                result = "already_running",
                "Admin job trigger skipped"
            );
            results.push(JobResult {
                job,
                error: Some("Job is already running".to_string()),
            });
            continue;
        }

        let result = match job.as_str() {
            "webhook_retry" => {
                let mut err = None;
                if let Err(e) = crate::webhooks::scheduler::retry_webhooks(db.as_ref()).await {
                    error!(
                        admin_subject = %admin.subject,
                        admin_org = ?admin.org_id,
                        action = "run_background_job",
                        job = "webhook_retry",
                        error = %e,
                        "Manual background job failed"
                    );
                    err = Some(e.to_string());
                }
                if let Err(e) =
                    crate::webhooks::scheduler::retry_google_play_subscription_acknowledgements(
                        db.as_ref(),
                    )
                    .await
                {
                    error!(
                        admin_subject = %admin.subject,
                        admin_org = ?admin.org_id,
                        action = "run_background_job",
                        job = "google_play_acknowledgement_retry",
                        provider = "google_play",
                        error = %e,
                        "Manual background job failed"
                    );
                    err = Some(e.to_string());
                }
                err
            }
            "reconciliation" => {
                match crate::webhooks::scheduler::reconcile_subscriptions(&db).await {
                    Ok(()) => None,
                    Err(e) => {
                        error!(
                            admin_subject = %admin.subject,
                            admin_org = ?admin.org_id,
                            action = "run_background_job",
                            job = "reconciliation",
                            error = %e,
                            "Manual background job failed"
                        );
                        Some(e.to_string())
                    }
                }
            }
            "price_step_up" => {
                match crate::webhooks::scheduler::process_price_step_up_expiry(&db).await {
                    Ok(()) => None,
                    Err(e) => {
                        error!(
                            admin_subject = %admin.subject,
                            admin_org = ?admin.org_id,
                            action = "run_background_job",
                            job = "price_step_up",
                            error = %e,
                            "Manual background job failed"
                        );
                        Some(e.to_string())
                    }
                }
            }
            "pause_scheduler" => {
                match crate::webhooks::scheduler::process_pause_transitions(&db).await {
                    Ok(()) => None,
                    Err(e) => {
                        error!(
                            admin_subject = %admin.subject,
                            admin_org = ?admin.org_id,
                            action = "run_background_job",
                            job = "pause_scheduler",
                            error = %e,
                            "Manual background job failed"
                        );
                        Some(e.to_string())
                    }
                }
            }
            "cleanup" => match crate::webhooks::scheduler::cleanup_old_data(&db).await {
                Ok(()) => None,
                Err(e) => {
                    error!(
                        admin_subject = %admin.subject,
                        admin_org = ?admin.org_id,
                        action = "run_background_job",
                        job = "cleanup",
                        error = %e,
                        "Manual background job failed"
                    );
                    Some(e.to_string())
                }
            },
            "webhook_retry_cleanup" => match crate::webhooks::scheduler::cleanup_webhook_retry(&db).await {
                Ok(()) => None,
                Err(e) => {
                    error!(
                        admin_subject = %admin.subject,
                        admin_org = ?admin.org_id,
                        action = "run_background_job",
                        job = "webhook_retry_cleanup",
                        error = %e,
                        "Manual background job failed"
                    );
                    Some(e.to_string())
                }
            },
            "reconciliation_cleanup" => {
                finish_admin_operation("trigger_job:reconciliation").await;
                None
            }
            "price_step_up_cleanup" => match crate::webhooks::scheduler::cleanup_price_step_up(&db).await {
                Ok(()) => None,
                Err(e) => {
                    error!(
                        admin_subject = %admin.subject,
                        admin_org = ?admin.org_id,
                        action = "run_background_job",
                        job = "price_step_up_cleanup",
                        error = %e,
                        "Manual background job failed"
                    );
                    Some(e.to_string())
                }
            },
            "pause_scheduler_cleanup" => match crate::webhooks::scheduler::cleanup_pause_scheduler(&db).await {
                Ok(()) => None,
                Err(e) => {
                    error!(
                        admin_subject = %admin.subject,
                        admin_org = ?admin.org_id,
                        action = "run_background_job",
                        job = "pause_scheduler_cleanup",
                        error = %e,
                        "Manual background job failed"
                    );
                    Some(e.to_string())
                }
            },
            "reset_stuck_workers" => match crate::webhooks::scheduler::reset_stuck_workers(&db).await {
                Ok(()) => None,
                Err(e) => {
                    error!(
                        admin_subject = %admin.subject,
                        admin_org = ?admin.org_id,
                        action = "run_background_job",
                        job = "reset_stuck_workers",
                        error = %e,
                        "Manual background job failed"
                    );
                    Some(e.to_string())
                }
            },
            _ => unreachable!("validated above"),
        };

        finish_admin_operation(&lock_key).await;

        if let Some(error) = result.as_deref() {
            error!(
                admin_subject = %admin.subject,
                admin_org = ?admin.org_id,
                action = "trigger_job",
                target_job = %job,
                result = "error",
                error = %error,
                "Admin job trigger failed"
            );
        } else {
            info!(
                admin_subject = %admin.subject,
                admin_org = ?admin.org_id,
                action = "trigger_job",
                target_job = %job,
                result = "success",
                "Admin job trigger completed"
            );
        }
        results.push(JobResult { job, error: result });
    }

    info!(
        "Admin trigger-jobs completed: {:?}",
        results.iter().map(|r| &r.job).collect::<Vec<_>>()
    );

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

#[derive(serde::Serialize)]
pub struct AlertSignalSummary {
    pub alert_key: &'static str,
    pub signal_class: &'static str,
    pub route: &'static str,
    pub alert_subject: &'static str,
    pub count: Option<i64>,
    pub source: &'static str,
    pub log_query: &'static str,
}

impl AlertSignalSummary {
    fn counted(
        alert_key: &'static str,
        signal_class: &'static str,
        route: &'static str,
        alert_subject: &'static str,
        count: i64,
        source: &'static str,
    ) -> Self {
        Self {
            alert_key,
            signal_class,
            route,
            alert_subject,
            count: Some(count),
            source,
            log_query: alert_key,
        }
    }

    fn log_routed(
        alert_key: &'static str,
        signal_class: &'static str,
        route: &'static str,
        alert_subject: &'static str,
    ) -> Self {
        Self {
            alert_key,
            signal_class,
            route,
            alert_subject,
            count: None,
            source: "Logs: query by alert_key",
            log_query: alert_key,
        }
    }
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
