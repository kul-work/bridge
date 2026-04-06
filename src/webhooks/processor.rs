use crate::error::BridgeError;
use lettre::{
    message::Mailbox,
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport,
    AsyncTransport,
    Message,
    Tokio1Executor,
};
use sqlx::PgPool;
use std::env;
use uuid::Uuid;
use tracing::{error, info, warn};

/// Webhook event type (canonical)
/// Used for future webhook event normalization and processing.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum WebhookEventType {
    SubscriptionCreated,
    SubscriptionRenewed,
    SubscriptionExpired,
    SubscriptionCancelled,
    PaymentFailed,
    PaymentSucceeded,
    Unknown(String),
}

/// Canonical webhook payload sent to apps
/// Used for webhook forwarding to app callbacks.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize)]
pub struct CanonicalWebhookPayload {
    pub event_id: String,
    pub event_type: String,
    pub timestamp: String,                    // ISO 8601 format
    pub timestamp_epoch_ms: i64,             // Unix epoch milliseconds
    pub app_slug: String,                    // from app lookup
    pub product_id: Option<String>,
    pub subscription_id: Option<String>,
    pub external_user_id: Option<String>,
    pub amount_cents: Option<i32>,
    pub auto_renewing: Option<bool>,
    pub purchase_token: Option<String>,
    pub current_period_end: Option<String>,  // ISO 8601
    pub status: Option<String>,
    pub provider: String,
    pub provider_event_id: String,
    pub previous_status: Option<String>,
    pub corrected_status: Option<String>,
    pub reconciliation_source: Option<String>,
    pub revocation_reason: Option<String>,
    pub cancellation_mode: Option<String>,
}

#[allow(dead_code)]
struct WebhookFields {
    subscription_id: Option<String>,
    purchase_token: Option<String>,
    amount_cents: Option<i32>,
    auto_renewing: Option<bool>,
    current_period_end: Option<String>,
    provider_transaction_id: Option<String>,
    provider_customer_id: Option<String>,
    product_id: Option<String>,
    cancel_reason: Option<String>,
    status: Option<String>,
    google_subscription_state: Option<i32>,
    google_cancellation_context: Option<String>,
    google_cancellation_feedback: Option<String>,
    google_new_price_cents: Option<i32>,
    google_price_step_up_consent_deadline: Option<String>,
}

fn extract_metadata_user_id(payload: &serde_json::Value) -> Option<String> {
    [
        "/metadata/user_id",
        "/object/metadata/user_id",
        "/event/data/metadata/external_user_id",
        "/event/data/metadata/user_id",
    ]
    .into_iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(|value| value.as_str()).map(|value| value.to_string()))
}

async fn suppress_unresolved_webhook(
    pool: &PgPool,
    webhook_provider_id: Uuid,
    webhook: &crate::db::webhooks::WebhookProvider,
) -> Result<(), BridgeError> {
    error!(
        "Webhook {} discarded: unable to resolve external_user_id (provider={}, event={})",
        webhook.provider_webhook_id,
        webhook.provider,
        webhook.event_type
    );
    crate::db::webhooks::suppress_webhook(pool, webhook_provider_id, "unresolved_external_user_id").await
}

async fn ensure_resolved_user(
    pool: &PgPool,
    webhook_provider_id: Uuid,
    webhook: &crate::db::webhooks::WebhookProvider,
    external_user_id: &Option<String>,
) -> Result<bool, BridgeError> {
    if external_user_id.is_some() {
        return Ok(true);
    }

    suppress_unresolved_webhook(pool, webhook_provider_id, webhook).await?;
    Ok(false)
}

fn extract_customer_email_for_dispute(webhook: &crate::db::webhooks::WebhookProvider) -> Option<String> {
    let payload = &webhook.payload;
    [
        "/customer/email",
        "/object/customer/email",
        "/object/customer_email",
        "/event/data/metadata/email",
        "/event/data/customer/email",
        "/data/attributes/customer_email",
        "/data/attributes/customer[email]",
    ]
    .into_iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(|value| value.as_str()).map(|value| value.to_string()))
}

async fn send_dispute_admin_alert_email(
    app: &crate::db::apps::App,
    webhook: &crate::db::webhooks::WebhookProvider,
    fields: &WebhookFields,
    external_user_id: Option<&str>,
) -> Result<(), BridgeError> {
    let admin_email = match env::var("ADMIN_ALERT_EMAIL")
        .or_else(|_| env::var("TYDE_SUPPORT_EMAIL"))
    {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            warn!(
                "Skipping dispute admin email for event {}: ADMIN_ALERT_EMAIL not configured",
                webhook.provider_webhook_id
            );
            return Ok(());
        }
    };

    let smtp_host = match env::var("SMTP_HOST") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            warn!(
                "Skipping dispute admin email for event {}: SMTP_HOST not configured",
                webhook.provider_webhook_id
            );
            return Ok(());
        }
    };

    let smtp_port = env::var("SMTP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(587);
    let from_email = env::var("SMTP_FROM_EMAIL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "noreply@bridge.local".to_string());

    let customer_email = extract_customer_email_for_dispute(webhook)
        .unwrap_or_else(|| "unknown".to_string());
    let amount_cents = fields.amount_cents.unwrap_or(0);
    let subscription_id = fields
        .subscription_id
        .as_deref()
        .or(webhook.subscription_id.as_deref())
        .unwrap_or("unknown");
    let external_user_id = external_user_id.unwrap_or("unknown");
    let subject = format!(
        "Bridge dispute created: {} ({})",
        app.display_name,
        webhook.provider_webhook_id
    );
    let body = format!(
        "A dispute webhook was received by Bridge.\n\n\
         App: {}\n\
         App slug: {}\n\
         Provider: {}\n\
         Event ID: {}\n\
         Event type: {}\n\
         Subscription ID: {}\n\
         External user ID: {}\n\
         Customer email: {}\n\
         Amount cents: {}\n\
         Timestamp: {}\n",
        app.display_name,
        app.slug,
        webhook.provider,
        webhook.provider_webhook_id,
        webhook.event_type,
        subscription_id,
        external_user_id,
        customer_email,
        amount_cents,
        webhook
            .timestamp_epoch_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
    );

    let from: Mailbox = from_email
        .parse()
        .map_err(|e| BridgeError::ConfigError(format!("Invalid SMTP_FROM_EMAIL: {}", e)))?;
    let to: Mailbox = admin_email
        .parse()
        .map_err(|e| BridgeError::ConfigError(format!("Invalid ADMIN_ALERT_EMAIL: {}", e)))?;

    let message = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .body(body)
        .map_err(|e| BridgeError::InternalServerError(format!("Failed to build admin alert email: {}", e)))?;

    let mut transport_builder = AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp_host)
        .map_err(|e| BridgeError::ConfigError(format!("Invalid SMTP_HOST '{}': {}", smtp_host, e)))?;
    transport_builder = transport_builder.port(smtp_port);

    if let (Ok(username), Ok(password)) = (env::var("SMTP_USERNAME"), env::var("SMTP_PASSWORD")) {
        if !username.trim().is_empty() && !password.trim().is_empty() {
            transport_builder = transport_builder.credentials(Credentials::new(username, password));
        }
    }

    let mailer = transport_builder.build();
    mailer
        .send(message)
        .await
        .map_err(|e| BridgeError::InternalServerError(format!("Failed to send dispute admin alert email: {}", e)))?;

    info!(
        "Dispute admin email sent for app_id={} provider_event_id={}",
        app.id,
        webhook.provider_webhook_id
    );

    Ok(())
}

fn extract_webhook_fields(webhook: &crate::db::webhooks::WebhookProvider) -> WebhookFields {
    let p = &webhook.payload;
    match webhook.provider.as_str() {
        "google_play" => WebhookFields {
            subscription_id: p.pointer("/subscriptionNotification/subscriptionId")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            purchase_token: p.pointer("/subscriptionNotification/purchaseToken")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            amount_cents: p.pointer("/oneTimeProductNotification/priceMicros")
                .and_then(|v| v.as_i64()).map(|m| (m / 10_000) as i32),
            auto_renewing: p.pointer("/subscriptionNotification/autoRenewing")
                .and_then(|v| v.as_bool()),
            current_period_end: p.pointer("/subscriptionNotification/expiryTimeMillis")
                .and_then(|v| v.as_i64())
                .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                .map(|dt| dt.to_rfc3339()),
            provider_transaction_id: p.pointer("/subscriptionNotification/purchaseToken")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            provider_customer_id: None,
            product_id: p.pointer("/subscriptionNotification/subscriptionId")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| p.pointer("/oneTimeProductNotification/sku")
                    .and_then(|v| v.as_str()).map(|s| s.to_string())),
            cancel_reason: p.pointer("/subscriptionNotification/cancelReason")
                .and_then(|v| v.as_i64()).map(|c| c.to_string()),
            status: None,
            google_subscription_state: None,
            google_cancellation_context: None,
            google_cancellation_feedback: None,
            google_new_price_cents: p.pointer("/subscriptionNotification/priceStepUpConsentDetails/priceMicros")
                .and_then(|v| v.as_i64()).map(|m| (m / 10_000) as i32),
            google_price_step_up_consent_deadline: p.pointer("/subscriptionNotification/priceStepUpConsentDetails/consentDeadlineTimeMillis")
                .and_then(|v| v.as_i64())
                .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                .map(|dt| dt.to_rfc3339()),
        },
        "creem" => WebhookFields {
            subscription_id: p.pointer("/object/subscription/id")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| p.pointer("/object/subscription_id")
                    .and_then(|v| v.as_str()).map(|s| s.to_string())),
            purchase_token: None,
            amount_cents: p.pointer("/object/amount")
                .and_then(|v| v.as_i64()).map(|a| a as i32),
            auto_renewing: None,
            current_period_end: p.pointer("/object/current_period_end")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            provider_transaction_id: p.pointer("/object/id")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            provider_customer_id: p.pointer("/object/customer/id")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            product_id: p.pointer("/object/product/id")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            cancel_reason: None,
            status: p.pointer("/object/status")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            google_subscription_state: None,
            google_cancellation_context: None,
            google_cancellation_feedback: None,
            google_new_price_cents: None,
            google_price_step_up_consent_deadline: None,
        },
        "lemonsqueezy" => WebhookFields {
            subscription_id: p.pointer("/data/id")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            purchase_token: None,
            amount_cents: p.pointer("/data/attributes/total")
                .and_then(|v| v.as_i64()).map(|a| a as i32),
            auto_renewing: None,
            current_period_end: p.pointer("/data/attributes/renews_at")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            provider_transaction_id: p.pointer("/data/id")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            provider_customer_id: p.pointer("/data/attributes/customer_id")
                .and_then(|v| v.as_i64()).map(|c| c.to_string()),
            product_id: p.pointer("/data/attributes/product_id")
                .and_then(|v| v.as_i64()).map(|c| c.to_string()),
            cancel_reason: None,
            status: p.pointer("/data/attributes/status")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            google_subscription_state: None,
            google_cancellation_context: None,
            google_cancellation_feedback: None,
            google_new_price_cents: None,
            google_price_step_up_consent_deadline: None,
        },
        "coinbase" => WebhookFields {
            subscription_id: None,
            purchase_token: None,
            amount_cents: p.pointer("/event/data/pricing/local/amount")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .map(|a| (a * 100.0) as i32),
            auto_renewing: None,
            current_period_end: None,
            provider_transaction_id: p.pointer("/event/data/id")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            provider_customer_id: None,
            product_id: p.pointer("/event/data/metadata/product_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            cancel_reason: None,
            status: None,
            google_subscription_state: None,
            google_cancellation_context: None,
            google_cancellation_feedback: None,
            google_new_price_cents: None,
            google_price_step_up_consent_deadline: None,
        },
        _ => WebhookFields {
            subscription_id: None,
            purchase_token: None,
            amount_cents: None,
            auto_renewing: None,
            current_period_end: None,
            provider_transaction_id: None,
            provider_customer_id: None,
            product_id: None,
            cancel_reason: None,
            status: None,
            google_subscription_state: None,
            google_cancellation_context: None,
            google_cancellation_feedback: None,
            google_new_price_cents: None,
            google_price_step_up_consent_deadline: None,
        },
    }
}

fn parse_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn google_subscription_state_to_status(subscription_state: Option<&str>) -> Option<String> {
    let state = subscription_state?;
    let normalized = match state {
        "SUBSCRIPTION_STATE_ACTIVE" => "active",
        "SUBSCRIPTION_STATE_CANCELED" => "cancelled",
        "SUBSCRIPTION_STATE_IN_GRACE_PERIOD" => "past_due",
        "SUBSCRIPTION_STATE_ON_HOLD" => "on_hold",
        "SUBSCRIPTION_STATE_PAUSED" => "paused",
        "SUBSCRIPTION_STATE_PENDING" => "pending",
        "SUBSCRIPTION_STATE_EXPIRED" => "expired",
        _ => return None,
    };

    Some(normalized.to_string())
}

fn google_cancellation_context_from_resource(
    resource: &crate::services::google_play::models::SubscriptionPurchaseV2,
) -> (Option<String>, Option<String>) {
    let Some(context) = resource.canceled_state_context.as_ref() else {
        return (None, None);
    };

    if let Some(user_cancelled) = context.user_initiated_cancellation.as_ref() {
        return (
            Some("user_initiated".to_string()),
            user_cancelled
                .cancel_survey_result
                .as_ref()
                .and_then(|survey| survey.reason_user_input.clone().or_else(|| survey.reason.clone())),
        );
    }

    if context.system_initiated_cancellation.is_some() {
        return (Some("system_initiated".to_string()), None);
    }

    if context.developer_initiated_cancellation.is_some() {
        return (Some("developer_initiated".to_string()), None);
    }

    if context.replacement_cancellation.is_some() {
        return (Some("replacement".to_string()), None);
    }

    (None, None)
}

async fn enrich_google_play_fields(
    pool: &PgPool,
    app_id: Uuid,
    webhook: &crate::db::webhooks::WebhookProvider,
    mut fields: WebhookFields,
) -> Result<WebhookFields, BridgeError> {
    let Some(purchase_token) = fields.purchase_token.as_deref().or(webhook.purchase_token.as_deref()) else {
        return Ok(fields);
    };

    let Some(subscription_id) = fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()) else {
        return Ok(fields);
    };

    let provider_config = match crate::db::provider_configs::get_provider_config(pool, app_id, "google_play").await {
        Ok(config) => config,
        Err(_) => return Ok(fields),
    };

    let package_name = provider_config
        .config
        .get("package_name")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let service_account_json = provider_config
        .config
        .get("service_account_json")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    if package_name.is_empty() || service_account_json.is_empty() {
        return Ok(fields);
    }

    let client = match crate::services::google_play::client::GooglePlayClient::new(service_account_json) {
        Ok(client) => client,
        Err(_) => return Ok(fields),
    };

    let Ok(resource) = client.get_subscription(package_name, subscription_id, purchase_token).await else {
        return Ok(fields);
    };

    if fields.current_period_end.is_none() {
        fields.current_period_end = resource.expiry_time.clone();
    }

    if fields.auto_renewing.is_none() {
        fields.auto_renewing = resource.auto_renewing;
    }

    if fields.product_id.is_none() {
        fields.product_id = resource.line_items.first().map(|line_item| line_item.product_id.clone());
    }

    if fields.status.is_none() {
        fields.status = google_subscription_state_to_status(resource.subscription_state.as_deref());
    }

    if fields.google_subscription_state.is_none() {
        fields.google_subscription_state = resource.subscription_state.as_deref().and_then(|state| match state {
            "SUBSCRIPTION_STATE_ACTIVE" => Some(0),
            "SUBSCRIPTION_STATE_CANCELED" => Some(1),
            "SUBSCRIPTION_STATE_IN_GRACE_PERIOD" => Some(2),
            "SUBSCRIPTION_STATE_ON_HOLD" => Some(3),
            "SUBSCRIPTION_STATE_PAUSED" => Some(4),
            "SUBSCRIPTION_STATE_PENDING" => Some(5),
            "SUBSCRIPTION_STATE_EXPIRED" => Some(6),
            _ => None,
        });
    }

    let (cancellation_context, cancellation_feedback) = google_cancellation_context_from_resource(&resource);
    if fields.google_cancellation_context.is_none() {
        fields.google_cancellation_context = cancellation_context;
    }
    if fields.google_cancellation_feedback.is_none() {
        fields.google_cancellation_feedback = cancellation_feedback;
    }

    Ok(fields)
}

/// Resolve external_user_id via §53 cascade
async fn resolve_user(
    pool: &PgPool,
    app_id: Uuid,
    webhook: &crate::db::webhooks::WebhookProvider,
) -> Option<String> {
    // 1. subscription_id lookup
    if let Some(ref sub_id) = webhook.subscription_id {
        if let Ok(Some(user)) = crate::db::subscriptions::lookup_user_by_subscription_id(pool, app_id, sub_id).await {
            return Some(user);
        }
    }

    // 2. purchase_token lookup (subscriptions first, then payments)
    if let Some(ref token) = webhook.purchase_token {
        if let Ok(Some(user)) = crate::db::subscriptions::lookup_user_by_purchase_token(pool, app_id, token).await {
            return Some(user);
        }
        if let Ok(Some(user)) = crate::db::payments::lookup_user_by_purchase_token_payment(pool, app_id, token).await {
            return Some(user);
        }
    }

    // 3. Google Play Strategy 3 (obfuscated_account_id lookup)
    if webhook.provider == "google_play" {
        if let Some(ref token) = webhook.purchase_token {
            if let Ok(config) = crate::db::provider_configs::get_provider_config(pool, app_id, "google_play").await {
                let pkg = config.config.get("package_name").and_then(|v| v.as_str()).unwrap_or("");
                let sa = config.config.get("service_account_json").and_then(|v| v.as_str()).unwrap_or("");
                
                if let Ok(gp_client) = crate::services::google_play::client::GooglePlayClient::new(sa) {
                    if let Ok(sub) = gp_client.get_subscription(pkg, "", token).await {
                        if let Some(ref ids) = sub.external_account_identifiers {
                            if let Some(ref obf_id) = ids.obfuscated_account_id {
                                if let Ok(Some(user)) = crate::db::subscriptions::lookup_user_by_google_obfuscated_id(pool, app_id, obf_id).await {
                                    return Some(user);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 4. Provider metadata.user_id / external_user_id
    if let Some(user_id) = extract_metadata_user_id(&webhook.payload) {
        return Some(user_id);
    }

    // 5. Creem orphan guard
    if webhook.provider == "creem" {
        error!(
            "Creem orphan guard: discarding webhook {} (event={})",
            webhook.provider_webhook_id, webhook.event_type
        );
        return None;
    }

    // 6. All strategies failed
    None
}

pub async fn build_canonical_payload(
    pool: &PgPool,
    webhook_provider_id: Uuid,
    app_id: Uuid,
) -> Result<Option<CanonicalWebhookPayload>, BridgeError> {
    let webhook = crate::db::webhooks::get_webhook_provider(pool, webhook_provider_id)
        .await?;

    if webhook.suppressed {
        info!(
            "Webhook {} already suppressed: {}",
            webhook_provider_id, webhook.suppressed_reason.as_deref().unwrap_or("unknown")
        );
        return Ok(None);
    }

    let app = crate::db::apps::get_app(pool, app_id)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let external_user_id = resolve_user(pool, app_id, &webhook).await;
    if !ensure_resolved_user(pool, webhook_provider_id, &webhook, &external_user_id).await? {
        return Ok(None);
    }

    let mut fields = extract_webhook_fields(&webhook);
    if webhook.provider == "google_play" {
        fields = enrich_google_play_fields(pool, app_id, &webhook, fields).await?;
    }
    let canonical_event = normalize_event_type(&webhook.provider, &webhook.event_type);
    let timestamp_epoch_ms = webhook
        .timestamp_epoch_ms
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let timestamp_iso = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();
    let canonical_subscription = if let Some(ref sub_id) = fields.subscription_id {
        crate::db::subscriptions::get_subscription_by_sub_id(pool, app_id, sub_id).await?
    } else if let Some(ref token) = fields.purchase_token {
        crate::db::subscriptions::get_subscription_by_purchase_token(pool, app_id, token).await?
    } else {
        None
    };

    let callback_event_type = match canonical_event.as_str() {
        "subscription.activated" | "subscription.renewed" | "subscription.recovered" | "subscription.created" | "subscription.trial_started" => "subscription.activated".to_string(),
        "subscription.grace_period" => "subscription.grace_period".to_string(),
        "subscription.revoked" => "subscription.revoked".to_string(),
        "subscription.on_hold" => "subscription.on_hold".to_string(),
        "subscription.paused" => "subscription.paused".to_string(),
        "subscription.resumed" => "subscription.resumed".to_string(),
        "subscription.cancellation_scheduled" | "subscription.pending_purchase_cancelled" => "subscription.cancelled".to_string(),
        "subscription.expired" => "subscription.expired".to_string(),
        "subscription.cancelled" => "subscription.cancelled".to_string(),
        "payment.pending" => "payment.pending".to_string(),
        "payment.failed" => "payment.failed".to_string(),
        "purchase.one_time" => "purchase.one_time".to_string(),
        "purchase.one_time_cancelled" => "purchase.one_time".to_string(),
        "payment.refunded" => "payment.refunded".to_string(),
        "dispute.created" => "dispute.created".to_string(),
        "subscription.updated" => fields
            .status
            .as_deref()
            .and_then(status_to_canonical_event)
            .unwrap_or_else(|| "subscription.updated".to_string()),
        other => other.to_string(),
    };

    let canonical_status = match callback_event_type.as_str() {
        "payment.pending" => Some("pending".to_string()),
        "payment.failed" => Some("failed".to_string()),
        "payment.refunded" => Some("refunded".to_string()),
        "purchase.one_time" => Some(if canonical_event == "purchase.one_time_cancelled" {
            "cancelled".to_string()
        } else if canonical_event == "purchase.one_time" {
            "completed".to_string()
        } else {
            canonical_subscription
                .as_ref()
                .map(|sub| sub.status.clone())
                .or_else(|| fields.status.clone())
                .unwrap_or_else(|| "completed".to_string())
        }),
        _ => canonical_subscription
            .as_ref()
            .map(|sub| sub.status.clone())
            .or_else(|| fields.status.clone())
            .or_else(|| match callback_event_type.as_str() {
                "subscription.activated" | "subscription.resumed" => Some("active".to_string()),
                "subscription.grace_period" => Some("past_due".to_string()),
                "subscription.revoked" => Some("revoked".to_string()),
                "subscription.on_hold" => Some("on_hold".to_string()),
                "subscription.paused" => Some("paused".to_string()),
                "subscription.expired" => Some("expired".to_string()),
                "subscription.cancelled" => Some("cancelled".to_string()),
                _ => None,
            }),
    };

    let canonical_current_period_end = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.current_period_end.map(|value| value.to_rfc3339()))
        .or_else(|| fields.current_period_end.clone());
    let canonical_auto_renewing = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.auto_renewing)
        .or(fields.auto_renewing);
    let canonical_purchase_token = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.purchase_token.clone())
        .or_else(|| fields.purchase_token.clone())
        .or_else(|| webhook.purchase_token.clone());
    let canonical_revocation_reason = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.revocation_reason.clone())
        .or_else(|| fields.cancel_reason.clone());

    Ok(Some(CanonicalWebhookPayload {
        event_id: format!("{}-{}", webhook.provider, webhook.provider_webhook_id),
        event_type: callback_event_type,
        timestamp: timestamp_iso,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: fields.product_id,
        subscription_id: fields.subscription_id.or(webhook.subscription_id.clone()),
        external_user_id,
        amount_cents: fields.amount_cents,
        auto_renewing: canonical_auto_renewing,
        purchase_token: canonical_purchase_token,
        current_period_end: canonical_current_period_end,
        status: canonical_status,
        provider: webhook.provider.clone(),
        provider_event_id: webhook.provider_webhook_id.clone(),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: canonical_revocation_reason,
        cancellation_mode: if canonical_event == "subscription.cancellation_scheduled" {
            Some("scheduled".to_string())
        } else {
            None
        },
    }))
}

/// Process webhook: dedup, ordering, normalization, DB mutations
#[allow(dead_code)]
pub async fn process_webhook(
    pool: &PgPool,
    webhook_provider_id: Uuid,
    app_id: Uuid,
) -> Result<Option<CanonicalWebhookPayload>, BridgeError> {
    // Step 1: Load webhook + app
    let webhook = crate::db::webhooks::get_webhook_provider(pool, webhook_provider_id)
        .await?;

    if webhook.suppressed {
        info!(
            "Webhook {} already suppressed: {}",
            webhook_provider_id, webhook.suppressed_reason.as_deref().unwrap_or("unknown")
        );
        return Ok(None);
    }

    let app = crate::db::apps::get_app(pool, app_id)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let canonical_event = normalize_event_type(&webhook.provider, &webhook.event_type);

    if webhook.provider == "coinbase" && canonical_event == "charge.failed" {
        let fields = extract_webhook_fields(&webhook);
        let charge_id = fields.provider_transaction_id
            .as_deref()
            .or(webhook.subscription_id.as_deref())
            .unwrap_or(&webhook.provider_webhook_id);
        let email = webhook.payload.pointer("/event/data/metadata/email")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");

        warn!("Coinbase charge failed: charge_id={}, email={}", charge_id, email);

        sqlx::query("UPDATE pay.webhook_provider SET processed = true WHERE id = $1")
            .bind(webhook_provider_id)
            .execute(pool)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;

        return Ok(None);
    }

    let external_user_id = resolve_user(pool, app_id, &webhook).await;
    if !ensure_resolved_user(pool, webhook_provider_id, &webhook, &external_user_id).await? {
        return Ok(None);
    }

    let mut fields = extract_webhook_fields(&webhook);
    if webhook.provider == "google_play" {
        fields = enrich_google_play_fields(pool, app_id, &webhook, fields).await?;
    }

    let timestamp_epoch_ms = webhook.timestamp_epoch_ms.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let timestamp_iso = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();
    let mut callback_event_type = canonical_event.clone();
    let mut callback_status_override: Option<String> = None;
    let mut callback_revocation_reason_override: Option<String> = None;
    let mut callback_cancellation_mode_override: Option<String> = None;
    let mut canonical_subscription: Option<crate::db::subscriptions::Subscription> = None;
    let mut should_forward = true;

    // Step 4: Route by canonical event type and mutate DB
    match canonical_event.as_str() {
        // §13 - Subscription Activation
        "subscription.activated" | "subscription.renewed" | "subscription.recovered" | "subscription.created" => {
            if let Some(ref user_id) = external_user_id {
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

                let period_end = fields.current_period_end.as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let sub_id_fallback = webhook.subscription_id.clone().unwrap_or_default();
                let sub_id_str = fields.subscription_id.as_deref()
                    .unwrap_or(&sub_id_fallback);

                let upsert_result = crate::db::subscriptions::upsert_subscription_tx(
                    &mut tx, app_id, user_id,
                    sub_id_str,
                    &webhook.provider, "active", period_end,
                    fields.purchase_token.as_deref(), fields.auto_renewing,
                    None, fields.provider_customer_id.as_deref(),
                    timestamp_epoch_ms,
                ).await?;

                if !upsert_result.applied {
                    tx.rollback().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                    info!(
                        "Skipped stale activation event for subscription {} (provider: {})",
                        sub_id_str,
                        webhook.provider
                    );
                } else {
                    if let Some(ref txn_id) = fields.provider_transaction_id {
                        crate::db::payments::record_payment_tx(
                            &mut tx, app_id, user_id, &webhook.provider, txn_id,
                            fields.subscription_id.as_deref(),
                            fields.amount_cents.unwrap_or(0), "success",
                        ).await?;
                    }

                    if webhook.provider == "creem" {
                        let _ = crate::db::payments::adopt_stale_payment(&mut tx, app_id, user_id, sub_id_str).await;
                    }

                    canonical_subscription = Some(upsert_result.subscription);
                    callback_event_type = "subscription.activated".to_string();
                    callback_status_override = Some("active".to_string());
                    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

                    if webhook.provider == "google_play" {
                        let _ = crate::db::subscriptions::link_replacement_subscriptions(pool, app_id, user_id, sub_id_str, timestamp_epoch_ms).await;
                    }
                }
            }
        }
        
        // §14 - Subscription Pending
        "subscription.pending" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.as_deref()
                    .or(webhook.subscription_id.as_deref())
                    .unwrap_or("");
                let result = sqlx::query(
                    "UPDATE pay.subscriptions
                     SET status = 'pending',
                         google_subscription_state = 5,
                         version = version + 1,
                         last_event_time = $1,
                         updated_at = NOW()
                     WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1"
                )
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(sub_id)
                .execute(pool)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;
                if result.rows_affected() == 0 {
                    info!("Skipped stale pending event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
            should_forward = false;
        }

        // §15 - Grace Period
        "subscription.trial_started" => {
            if let Some(ref user_id) = external_user_id {
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

                let period_end = fields.current_period_end.as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let sub_id_fallback = webhook.subscription_id.clone().unwrap_or_default();
                let sub_id_str = fields.subscription_id.as_deref()
                    .unwrap_or(&sub_id_fallback);

                let upsert_result = crate::db::subscriptions::upsert_subscription_tx(
                    &mut tx, app_id, user_id,
                    sub_id_str,
                    &webhook.provider, "trial", period_end,
                    fields.purchase_token.as_deref(), fields.auto_renewing,
                    None, fields.provider_customer_id.as_deref(),
                    timestamp_epoch_ms,
                ).await?;

                if !upsert_result.applied {
                    tx.rollback().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                    info!(
                        "Skipped stale trial-start event for subscription {} (provider: {})",
                        sub_id_str,
                        webhook.provider
                    );
                } else {
                    canonical_subscription = Some(upsert_result.subscription);
                    callback_event_type = "subscription.activated".to_string();
                    callback_status_override = Some("trial".to_string());
                    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                }
            }
        }

        "subscription.grace_period" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                let grace_end = fields
                    .current_period_end
                    .as_deref()
                    .and_then(parse_rfc3339_utc);
                let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                    "UPDATE pay.subscriptions
                     SET status = 'past_due',
                         google_grace_period_start = COALESCE(google_grace_period_start, NOW()),
                         google_grace_period_end = COALESCE($1, google_grace_period_end),
                         google_subscription_state = 2,
                         version = version + 1,
                         last_event_time = $2,
                         updated_at = NOW()
                     WHERE app_id = $3 AND subscription_id = $4 AND last_event_time < $2
                     RETURNING *"
                )
                .bind(grace_end)
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(&sub_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;
                if let Some(sub) = updated {
                    canonical_subscription = Some(sub);
                    callback_event_type = "subscription.grace_period".to_string();
                    callback_status_override = Some("past_due".to_string());
                } else {
                    info!("Skipped stale grace_period event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        // §16 - Revoked
        "subscription.revoked" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.as_deref()
                    .or(webhook.subscription_id.as_deref())
                    .unwrap_or("");
                if let Some(token) = fields.purchase_token.as_deref().or(webhook.purchase_token.as_deref()) {
                    if crate::db::payments::get_payment_status(pool, app_id, token).await?.as_deref() == Some("refunded") {
                        info!("Skipped revoked event for subscription {} because payment {} is already refunded", sub_id, token);
                        return Ok(None);
                    }
                }

                let revocation_reason = fields
                    .cancel_reason
                    .clone()
                    .or_else(|| fields.google_cancellation_context.clone())
                    .unwrap_or_else(|| "unknown".to_string());

                let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                    "UPDATE pay.subscriptions
                     SET status = 'revoked',
                         auto_renewing = false,
                         revoked_at = NOW(),
                         revocation_reason = COALESCE($1, revocation_reason),
                         google_subscription_state = 6,
                         version = version + 1,
                         last_event_time = $2,
                         updated_at = NOW()
                     WHERE app_id = $3 AND subscription_id = $4 AND last_event_time < $2
                     RETURNING *"
                )
                .bind(Some(revocation_reason.clone()))
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(sub_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;
                if let Some(sub) = updated {
                    canonical_subscription = Some(sub);
                    callback_event_type = "subscription.revoked".to_string();
                    callback_status_override = Some("revoked".to_string());
                    callback_revocation_reason_override = Some(revocation_reason);
                } else {
                    info!("Skipped stale revoked event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        // §17 - On Hold
        "subscription.on_hold" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.as_deref()
                    .or(webhook.subscription_id.as_deref())
                    .unwrap_or("");
                let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                    "UPDATE pay.subscriptions
                     SET status = 'on_hold',
                         google_subscription_state = 3,
                         version = version + 1,
                         last_event_time = $1,
                         updated_at = NOW()
                     WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1
                     RETURNING *"
                )
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(sub_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;
                if let Some(sub) = updated {
                    canonical_subscription = Some(sub);
                    callback_event_type = "subscription.on_hold".to_string();
                    callback_status_override = Some("on_hold".to_string());
                } else {
                    info!("Skipped stale on_hold event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        // §18 - Paused (guard: only if current status is active or trial)
        "subscription.paused" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                if let Ok(Some(sub)) = crate::db::subscriptions::get_subscription_by_sub_id(pool, app_id, &sub_id).await {
                    if sub.status == "active" || sub.status == "trial" {
                        let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                            "UPDATE pay.subscriptions
                             SET status = 'paused',
                                 auto_renewing = false,
                                 google_paused_at = NOW(),
                                 google_subscription_state = 4,
                                 version = version + 1,
                                 last_event_time = $1,
                                 updated_at = NOW()
                             WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1
                             RETURNING *"
                        )
                        .bind(timestamp_epoch_ms)
                        .bind(app_id)
                        .bind(&sub_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| BridgeError::DbError(e.to_string()))?;
                        if let Some(updated_sub) = updated {
                            canonical_subscription = Some(updated_sub);
                            callback_event_type = "subscription.paused".to_string();
                            callback_status_override = Some("paused".to_string());
                        } else {
                            info!("Skipped stale paused event for subscription {}", sub_id);
                            return Ok(None);
                        }
                    } else {
                        info!("Ignoring pause event for subscription {} in status '{}' (not active/trial)", sub_id, sub.status);
                    }
                }
            }
        }

        // §19 - Restarted/Resumed (guard: only if current status is paused)
        "subscription.resumed" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                if let Ok(Some(sub)) = crate::db::subscriptions::get_subscription_by_sub_id(pool, app_id, &sub_id).await {
                    if sub.status == "paused" {
                        let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                            "UPDATE pay.subscriptions
                             SET status = 'active',
                                 auto_renewing = true,
                                 google_paused_at = NULL,
                                 cancellation_initiated_at = NULL,
                                 google_subscription_state = 0,
                                 version = version + 1,
                                 last_event_time = $1,
                                 updated_at = NOW()
                             WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1
                             RETURNING *"
                        )
                        .bind(timestamp_epoch_ms)
                        .bind(app_id)
                        .bind(&sub_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| BridgeError::DbError(e.to_string()))?;
                        if let Some(updated_sub) = updated {
                            canonical_subscription = Some(updated_sub);
                            callback_event_type = "subscription.resumed".to_string();
                            callback_status_override = Some("active".to_string());
                        } else {
                            info!("Skipped stale resumed event for subscription {}", sub_id);
                            return Ok(None);
                        }
                    } else {
                        info!("Ignoring resume event for subscription {} in status '{}' (not paused)", sub_id, sub.status);
                    }
                }
            }
        }

        // §20 - Cancellation Scheduled
        "subscription.cancellation_scheduled" => {
            if let Some(ref _user_id) = external_user_id {
                if let Some(sub_id) = fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()) {
                    let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                        "UPDATE pay.subscriptions
                         SET auto_renewing = false,
                             cancellation_initiated_at = COALESCE(cancellation_initiated_at, NOW()),
                             google_pending_cancellation = true,
                             google_pending_cancellation_at = COALESCE(google_pending_cancellation_at, NOW()),
                             google_cancellation_context = COALESCE($1, google_cancellation_context),
                             google_cancellation_feedback = COALESCE($2, google_cancellation_feedback),
                             version = version + 1,
                             last_event_time = $3,
                             updated_at = NOW()
                         WHERE app_id = $4 AND subscription_id = $5 AND last_event_time < $3
                         RETURNING *"
                    )
                    .bind(fields.google_cancellation_context.as_deref())
                    .bind(fields.google_cancellation_feedback.as_deref())
                    .bind(timestamp_epoch_ms)
                    .bind(app_id)
                    .bind(sub_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| BridgeError::DbError(e.to_string()))?;
                    if let Some(updated_sub) = updated {
                        canonical_subscription = Some(updated_sub);
                        callback_event_type = "subscription.cancelled".to_string();
                        callback_status_override = Some("active".to_string());
                        callback_cancellation_mode_override = Some("scheduled".to_string());
                    } else {
                        info!("Skipped stale cancellation_scheduled event for subscription {}", sub_id);
                        return Ok(None);
                    }
                }
            }
        }

        // §21 - Expired/Inactive
        "subscription.expired" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                    "UPDATE pay.subscriptions
                     SET status = 'expired',
                         auto_renewing = false,
                         google_subscription_state = 6,
                         version = version + 1,
                         last_event_time = $1,
                         updated_at = NOW()
                     WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1
                     RETURNING *"
                )
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(&sub_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;
                if let Some(updated_sub) = updated {
                    canonical_subscription = Some(updated_sub);
                    callback_event_type = "subscription.expired".to_string();
                    callback_status_override = Some("expired".to_string());
                } else {
                    info!("Skipped stale expired event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        // §22 - Cancelled
        "subscription.cancelled" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                let cancel_period_end = fields
                    .current_period_end
                    .as_deref()
                    .and_then(parse_rfc3339_utc);
                let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                    "UPDATE pay.subscriptions
                     SET status = 'cancelled',
                         auto_renewing = false,
                         current_period_end = COALESCE($1, current_period_end),
                         cancellation_initiated_at = COALESCE(cancellation_initiated_at, NOW()),
                         google_subscription_state = 1,
                         google_cancellation_context = COALESCE($2, google_cancellation_context),
                         google_cancellation_feedback = COALESCE($3, google_cancellation_feedback),
                         version = version + 1,
                         last_event_time = $4,
                         updated_at = NOW()
                     WHERE app_id = $5 AND subscription_id = $6 AND last_event_time < $4
                     RETURNING *"
                )
                .bind(cancel_period_end)
                .bind(fields.google_cancellation_context.as_deref())
                .bind(fields.google_cancellation_feedback.as_deref())
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(&sub_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;
                if let Some(updated_sub) = updated {
                    canonical_subscription = Some(updated_sub);
                    callback_event_type = "subscription.cancelled".to_string();
                    callback_status_override = Some("cancelled".to_string());
                } else {
                    info!("Skipped stale cancelled event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        // §23 - Order Created (payment pending)
        "payment.pending" => {
            if let Some(ref user_id) = external_user_id {
                let txn_id = fields.provider_transaction_id.as_deref()
                    .or(fields.subscription_id.as_deref())
                    .unwrap_or(&webhook.provider_webhook_id);
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                crate::db::payments::record_payment_tx(
                    &mut tx, app_id, user_id, &webhook.provider, txn_id,
                    fields.subscription_id.as_deref(),
                    fields.amount_cents.unwrap_or(0), "pending",
                ).await?;
                tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            }
            callback_event_type = "payment.pending".to_string();
            callback_status_override = Some("pending".to_string());
        }

        // §24 - Order Failed
        "payment.failed" if webhook.provider == "coinbase" => {
            let charge_id = fields.provider_transaction_id
                .as_deref()
                .or(webhook.subscription_id.as_deref())
                .unwrap_or(&webhook.provider_webhook_id);
            info!("Coinbase charge failed: charge_id={}", charge_id);
            callback_event_type = "payment.failed".to_string();
            callback_status_override = Some("failed".to_string());
        }

        "payment.failed" => {
            if let Some(ref user_id) = external_user_id {
                let sub_id = fields
                    .subscription_id
                    .as_deref()
                    .or(webhook.subscription_id.as_deref())
                    .unwrap_or("");

                if sub_id.is_empty() {
                    warn!(
                        "Skipping order.failed event {}: missing subscription_id",
                        webhook.provider_webhook_id
                    );
                    return Ok(None);
                }

                if crate::db::subscriptions::get_subscription_by_sub_id(pool, app_id, sub_id).await?.is_none() {
                    warn!(
                        "Skipping order.failed event {}: subscription {} not found",
                        webhook.provider_webhook_id,
                        sub_id
                    );
                    return Ok(None);
                }

                let txn_id = fields.provider_transaction_id.as_deref()
                    .or(fields.subscription_id.as_deref())
                    .unwrap_or(&webhook.provider_webhook_id);
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                crate::db::payments::record_payment_tx(
                    &mut tx, app_id, user_id, &webhook.provider, txn_id,
                    Some(sub_id),
                    fields.amount_cents.unwrap_or(0), "failed",
                ).await?;
                let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                    "UPDATE pay.subscriptions
                     SET payment_failure_notification = true,
                         version = version + 1,
                         last_event_time = CASE WHEN last_event_time < $1 THEN $1 ELSE last_event_time END,
                         updated_at = NOW()
                     WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1
                     RETURNING *"
                )
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(sub_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;

                if let Some(updated_sub) = updated {
                    canonical_subscription = Some(updated_sub);
                } else {
                    warn!(
                        "Skipping stale order.failed update for subscription {}",
                        sub_id
                    );
                    tx.rollback().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                    return Ok(None);
                }

                tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            }
            callback_event_type = "payment.failed".to_string();
            callback_status_override = Some("failed".to_string());
        }

        // §25 - One-Time Product Purchased
        "purchase.one_time" => {
            if let Some(ref user_id) = external_user_id {
                let txn_id = fields.purchase_token.as_deref()
                    .or(fields.provider_transaction_id.as_deref())
                    .unwrap_or(&webhook.provider_webhook_id);
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                crate::db::payments::record_payment_tx(
                    &mut tx, app_id, user_id, &webhook.provider, txn_id,
                    fields.subscription_id.as_deref(),
                    fields.amount_cents.unwrap_or(0), "success",
                ).await?;
                tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            }
            callback_event_type = "purchase.one_time".to_string();
            callback_status_override = Some("completed".to_string());
        }

        // §26 - One-Time Product Cancelled
        "purchase.one_time_cancelled" => {
            if let Some(ref user_id) = external_user_id {
                let token = fields.purchase_token.as_deref()
                    .or(fields.provider_transaction_id.as_deref())
                    .unwrap_or("");
                let existing = crate::db::payments::get_payment_status(pool, app_id, token).await?;
                if matches!(existing.as_deref(), Some("refunded") | Some("cancelled")) {
                    info!("Skipping duplicate one-time cancellation for payment {}", token);
                    return Ok(None);
                }
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                crate::db::payments::record_payment_tx(
                    &mut tx, app_id, user_id, &webhook.provider, token,
                    fields.subscription_id.as_deref(),
                    fields.amount_cents.unwrap_or(0), "cancelled",
                ).await?;
                tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            }
            callback_event_type = "purchase.one_time".to_string();
            callback_status_override = Some("cancelled".to_string());
        }

        // §27 - Purchase Voided (Refund)
        "payment.refunded" => {
            if let Some(ref _user_id) = external_user_id {
                if let Some(ref token) = fields.purchase_token {
                    let existing = crate::db::payments::get_payment_status(pool, app_id, token).await?;
                    if existing.as_deref() != Some("refunded") {
                        crate::db::payments::update_payment_status(pool, app_id, token, "refunded").await?;
                    }
                    if let Some(sub) = crate::db::subscriptions::get_subscription_by_purchase_token(pool, app_id, token).await? {
                        let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                            "UPDATE pay.subscriptions
                             SET status = 'revoked',
                                 auto_renewing = false,
                                 revoked_at = NOW(),
                                 revocation_reason = 'REFUND',
                                 google_subscription_state = 6,
                                 version = version + 1,
                                 last_event_time = $1,
                                 updated_at = NOW()
                             WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1
                             RETURNING *"
                        )
                        .bind(timestamp_epoch_ms)
                        .bind(app_id)
                        .bind(&sub.subscription_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| BridgeError::DbError(e.to_string()))?;
                        if let Some(updated_sub) = updated {
                            canonical_subscription = Some(updated_sub);
                        }
                    }
                } else if let Some(ref sub_id) = fields.subscription_id {
                    let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                        "UPDATE pay.subscriptions
                         SET status = 'revoked',
                             auto_renewing = false,
                             revoked_at = NOW(),
                             revocation_reason = 'REFUND',
                             google_subscription_state = 6,
                             version = version + 1,
                             last_event_time = $1,
                             updated_at = NOW()
                         WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1
                         RETURNING *"
                    )
                    .bind(timestamp_epoch_ms)
                    .bind(app_id)
                    .bind(sub_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| BridgeError::DbError(e.to_string()))?;
                    if let Some(updated_sub) = updated {
                        canonical_subscription = Some(updated_sub);
                    }
                }
            }
            callback_event_type = "payment.refunded".to_string();
            callback_status_override = Some("refunded".to_string());
        }

        // §42 - Coinbase charge confirmed (agent topup)
        "charge.confirmed" if webhook.provider == "coinbase" => {
            let charge_id = fields.provider_transaction_id
                .clone()
                .or_else(|| webhook.subscription_id.clone())
                .unwrap_or_else(|| webhook.provider_webhook_id.clone());

            let external_user_id = webhook.payload.pointer("/event/data/metadata/external_user_id")
                .and_then(|v| v.as_str())
                .or_else(|| webhook.payload.pointer("/event/data/metadata/user_id").and_then(|v| v.as_str()))
                .map(|s| s.to_string());

            let amount_cents = fields.amount_cents
                .or_else(|| {
                    webhook.payload.pointer("/event/data/metadata/amount_cents")
                        .and_then(|v| v.as_i64())
                        .map(|v| v as i32)
                })
                .unwrap_or(0);

            if amount_cents <= 0 {
                info!("Coinbase charge {} skipped: non-positive amount {}", charge_id, amount_cents);
            } else if let Some(user_id) = external_user_id {
                let inserted = crate::db::agent::apply_topup_if_new(
                    pool,
                    app_id,
                    &user_id,
                    amount_cents,
                    &charge_id,
                )
                .await?;

                if inserted {
                    info!("Coinbase topup applied: charge_id={}, user={}, amount_cents={}", charge_id, user_id, amount_cents);
                } else {
                    info!("Coinbase topup already applied (idempotent): charge_id={}", charge_id);
                }
            } else {
                info!("Coinbase charge {} skipped: missing metadata external_user_id/user_id", charge_id);
            }
        }

        "subscription.pending_purchase_cancelled" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                let updated = sqlx::query_as::<_, crate::db::subscriptions::Subscription>(
                    "UPDATE pay.subscriptions
                     SET status = 'cancelled',
                         auto_renewing = false,
                         cancellation_initiated_at = COALESCE(cancellation_initiated_at, NOW()),
                         revocation_reason = 'pending_purchase_canceled',
                         google_subscription_state = 1,
                         version = version + 1,
                         last_event_time = $1,
                         updated_at = NOW()
                     WHERE app_id = $2 AND subscription_id = $3 AND last_event_time < $1
                     RETURNING *"
                )
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(&sub_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;
                if let Some(updated_sub) = updated {
                    canonical_subscription = Some(updated_sub);
                    callback_event_type = "subscription.cancelled".to_string();
                    callback_status_override = Some("cancelled".to_string());
                } else {
                    info!("Skipped stale pending_purchase_cancelled event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        // §29 - Dispute Created (admin alert + app callback)
        "dispute.created" => {
            if let Err(e) = send_dispute_admin_alert_email(&app, &webhook, &fields, external_user_id.as_deref()).await {
                warn!(
                    "Failed to send dispute admin alert for event {}: {}",
                    webhook.provider_webhook_id,
                    e
                );
            }

            // Forward callback to app with dispute notification
            if let Some(ref user_id) = external_user_id {
                let txn_id = fields.provider_transaction_id.as_deref()
                    .or(webhook.subscription_id.as_deref())
                    .unwrap_or(&webhook.provider_webhook_id);
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                crate::db::payments::record_payment_tx(
                    &mut tx, app_id, user_id, &webhook.provider, txn_id,
                    fields.subscription_id.as_deref(),
                    fields.amount_cents.unwrap_or(0), "dispute_created",
                ).await?;
                tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            }
            callback_event_type = "dispute.created".to_string();
        }


        "subscription.updated" => {
            if let Some(ref _user_id) = external_user_id {
                let status = normalize_status(fields.status.as_deref());
                let sub_id = fields.subscription_id.clone()
                    .or(webhook.subscription_id.clone())
                    .unwrap_or_default();

                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                let upsert_result = crate::db::subscriptions::upsert_subscription_tx(
                    &mut tx,
                    app_id,
                    _user_id,
                    &sub_id,
                    &webhook.provider,
                    &status,
                    fields
                        .current_period_end
                        .as_deref()
                        .and_then(parse_rfc3339_utc),
                    fields.purchase_token.as_deref(),
                    fields.auto_renewing,
                    None,
                    fields.provider_customer_id.as_deref(),
                    timestamp_epoch_ms,
                ).await?;

                if upsert_result.applied {
                    canonical_subscription = Some(upsert_result.subscription);
                    if let Some(new_event) = status_to_canonical_event(&status) {
                        callback_event_type = new_event;
                    }
                    callback_status_override = Some(status.clone());
                    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                } else {
                    tx.rollback().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                    info!("Skipped stale subscription.updated event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        "subscription.price_step_up" => {
            if let Some(sub_id) = fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()) {
                let deadline = fields.google_price_step_up_consent_deadline.as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let result = sqlx::query(
                    "UPDATE pay.subscriptions
                     SET google_requires_price_step_up_consent = true,
                         google_new_price_cents = $1,
                         google_price_step_up_consent_deadline = COALESCE($2, google_price_step_up_consent_deadline),
                         version = version + 1,
                         last_event_time = $3,
                         updated_at = NOW()
                     WHERE app_id = $4 AND subscription_id = $5 AND last_event_time < $3"
                )
                .bind(fields.google_new_price_cents)
                .bind(deadline)
                .bind(timestamp_epoch_ms)
                .bind(app_id)
                .bind(sub_id)
                .execute(pool)
                .await
                .map_err(|e| BridgeError::DbError(e.to_string()))?;

                if result.rows_affected() == 0 {
                    info!("Skipped stale price_step_up event for subscription {}", sub_id);
                }
            }
        }

        "subscription.pause_scheduled" => {
            if let Some(sub_id) = fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()) {
                let pause_scheduled_at = webhook.payload.pointer("/subscriptionNotification/pauseScheduleTimeMillis")
                    .and_then(|v| v.as_i64())
                    .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis);

                if let Some(schedule_at) = pause_scheduled_at {
                    let result = sqlx::query(
                        "UPDATE pay.subscriptions
                         SET google_pause_scheduled_at = $1,
                             version = version + 1,
                             last_event_time = $2,
                             updated_at = NOW()
                         WHERE app_id = $3 AND subscription_id = $4 AND last_event_time < $2"
                    )
                    .bind(schedule_at)
                    .bind(timestamp_epoch_ms)
                    .bind(app_id)
                    .bind(sub_id)
                    .execute(pool)
                    .await
                    .map_err(|e| BridgeError::DbError(e.to_string()))?;

                    if result.rows_affected() == 0 {
                        info!("Skipped stale pause_scheduled event for subscription {}", sub_id);
                    }
                }
            }
        }

        "subscription.deferred" => {
            if let Some(sub_id) = fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()) {
                let deferred_until = webhook.payload.pointer("/subscriptionNotification/deferredExpiryTimeMillis")
                    .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
                    .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis);
                if let Some(until) = deferred_until {
                    let result = sqlx::query(
                        "UPDATE pay.subscriptions
                         SET google_deferred_until = $1,
                             version = version + 1,
                             last_event_time = $2,
                             updated_at = NOW()
                         WHERE app_id = $3 AND subscription_id = $4 AND last_event_time < $2"
                    )
                    .bind(until)
                    .bind(timestamp_epoch_ms)
                    .bind(app_id)
                    .bind(sub_id)
                    .execute(pool)
                    .await
                    .map_err(|e| BridgeError::DbError(e.to_string()))?;

                    if result.rows_affected() == 0 {
                        info!("Skipped stale deferred event for subscription {}", sub_id);
                    }
                }
            }
        }

        "subscription.price_changed" => {
            if let Some(ref user_id) = external_user_id {
                let txn_id = fields.provider_transaction_id.as_deref()
                    .or(fields.subscription_id.as_deref())
                    .unwrap_or(&webhook.provider_webhook_id);
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                let _ = crate::db::payments::record_payment_tx(
                    &mut tx,
                    app_id,
                    user_id,
                    &webhook.provider,
                    txn_id,
                    fields.subscription_id.as_deref(),
                    fields.amount_cents.unwrap_or(0),
                    "price_changed",
                ).await;
                tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            }
        }

        "subscription.price_change_updated" | "subscription.expired_voided" => {
            info!("Processed informational webhook event: {} (provider: {})", canonical_event, webhook.provider);
        }

        "payment.succeeded" if webhook.provider == "coinbase" => {
            let charge_id = fields.provider_transaction_id
                .clone()
                .or_else(|| webhook.subscription_id.clone())
                .unwrap_or_else(|| webhook.provider_webhook_id.clone());

            let external_user_id = webhook.payload.pointer("/event/data/metadata/external_user_id")
                .and_then(|v| v.as_str())
                .or_else(|| webhook.payload.pointer("/event/data/metadata/user_id").and_then(|v| v.as_str()))
                .map(|s| s.to_string());

            let amount_cents = fields.amount_cents
                .or_else(|| {
                    webhook.payload.pointer("/event/data/metadata/amount_cents")
                        .and_then(|v| v.as_i64())
                        .map(|v| v as i32)
                })
                .unwrap_or(0);

            if amount_cents <= 0 {
                info!("Coinbase charge {} skipped: non-positive amount {}", charge_id, amount_cents);
            } else if let Some(user_id) = external_user_id {
                let inserted = crate::db::agent::apply_topup_if_new(
                    pool,
                    app_id,
                    &user_id,
                    amount_cents,
                    &charge_id,
                )
                .await?;

                if inserted {
                    info!("Coinbase topup applied: charge_id={}, user={}, amount_cents={}", charge_id, user_id, amount_cents);
                } else {
                    info!("Coinbase topup already applied (idempotent): charge_id={}", charge_id);
                }
            } else {
                info!("Coinbase charge {} skipped: missing metadata external_user_id/user_id", charge_id);
            }
        }

        // Unknown/unhandled
        _ => {
            info!("Unhandled webhook event type: {} (provider: {})", canonical_event, webhook.provider);
        }
    }

    if !should_forward {
        sqlx::query("UPDATE pay.webhook_provider SET processed = true WHERE id = $1")
            .bind(webhook_provider_id)
            .execute(pool)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;
        return Ok(None);
    }

    // Step 5: Build canonical payload with real data
    let canonical_status = callback_status_override
        .or_else(|| canonical_subscription.as_ref().map(|sub| sub.status.clone()))
        .or_else(|| fields.status.clone());
    let canonical_current_period_end = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.current_period_end.map(|value| value.to_rfc3339()))
        .or_else(|| fields.current_period_end.clone());
    let canonical_auto_renewing = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.auto_renewing)
        .or(fields.auto_renewing);
    let canonical_purchase_token = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.purchase_token.clone())
        .or_else(|| fields.purchase_token.clone())
        .or_else(|| webhook.purchase_token.clone());
    let canonical_revocation_reason = callback_revocation_reason_override
        .or_else(|| canonical_subscription.as_ref().and_then(|sub| sub.revocation_reason.clone()))
        .or_else(|| fields.cancel_reason.clone());
    let canonical = CanonicalWebhookPayload {
        event_id: format!("{}-{}", webhook.provider, webhook.provider_webhook_id),
        event_type: callback_event_type,
        timestamp: timestamp_iso,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: fields.product_id,
        subscription_id: fields.subscription_id.or(webhook.subscription_id.clone()),
        external_user_id,
        amount_cents: fields.amount_cents,
        auto_renewing: canonical_auto_renewing,
        purchase_token: canonical_purchase_token,
        current_period_end: canonical_current_period_end,
        status: canonical_status,
        provider: webhook.provider.clone(),
        provider_event_id: webhook.provider_webhook_id.clone(),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: canonical_revocation_reason,
        cancellation_mode: callback_cancellation_mode_override,
    };

    // Step 6: Mark webhook as processed
    sqlx::query("UPDATE pay.webhook_provider SET processed = true WHERE id = $1")
        .bind(webhook_provider_id)
        .execute(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(Some(canonical))
}

/// Normalize provider-specific event type to canonical format
/// Maps provider events per architecture doc section 3.4
#[allow(dead_code)]
fn normalize_event_type(provider: &str, event_type: &str) -> String {
    match provider {
        "google_play" => match event_type {
            // Google Play subscription notifications
            "SUBSCRIPTION_PURCHASED" => "subscription.activated".to_string(),
            "SUBSCRIPTION_RENEWED" => "subscription.renewed".to_string(),
            "SUBSCRIPTION_CANCELED" => "subscription.cancelled".to_string(),
            "SUBSCRIPTION_RESTORED" => "subscription.recovered".to_string(),
            "SUBSCRIPTION_EXPIRED" => "subscription.expired".to_string(),
            "SUBSCRIPTION_ON_HOLD" => "subscription.on_hold".to_string(),
            "SUBSCRIPTION_IN_GRACE_PERIOD" => "subscription.grace_period".to_string(),
            "SUBSCRIPTION_RESTARTED" => "subscription.resumed".to_string(),
            "SUBSCRIPTION_PAUSED" => "subscription.paused".to_string(),
            "SUBSCRIPTION_REVOKED" => "subscription.revoked".to_string(),
            "SUBSCRIPTION_DEFERRED" => "subscription.deferred".to_string(),
            "SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED" => "subscription.pause_scheduled".to_string(),
            "SUBSCRIPTION_RENEWAL_PENDING" => "subscription.pending".to_string(),
            "SUBSCRIPTION_PRICE_CHANGE_CONFIRMED" => "subscription.price_changed".to_string(),
            "SUBSCRIPTION_PRICE_CHANGE_UPDATED" => "subscription.price_change_updated".to_string(),
            "SUBSCRIPTION_PRICE_STEP_UP_CONSENT_UPDATED" => "subscription.price_step_up".to_string(),
            "SUBSCRIPTION_PENDING_PURCHASE_CANCELED" => "subscription.pending_purchase_cancelled".to_string(),
            "ONE_TIME_PRODUCT_PURCHASED" => "purchase.one_time".to_string(),
            "ONE_TIME_PRODUCT_REFUNDED" => "payment.refunded".to_string(),
            "ONE_TIME_PRODUCT_CANCELED" => "purchase.one_time_cancelled".to_string(),
            "VOIDED_PURCHASE" => "payment.refunded".to_string(),
            _ => format!("google_play.{}", event_type),
        },
        "creem" => match event_type {
            // Creem subscription events
            "subscription.created" => "subscription.created".to_string(),
            "subscription.active" => "subscription.activated".to_string(),
            "subscription.paid" => "subscription.activated".to_string(),
            "subscription.trialing" => "subscription.trial_started".to_string(),
            "subscription.past_due" => "subscription.grace_period".to_string(),
            "subscription.paused" => "subscription.paused".to_string(),
            "subscription.cancelled" => "subscription.cancelled".to_string(),
            "subscription.expired" => "subscription.expired".to_string(),
            "subscription.renewed" => "subscription.renewed".to_string(),
            "order.created" => "payment.pending".to_string(),
            "order.failed" => "payment.failed".to_string(),
            "order.completed" => "subscription.activated".to_string(),
            "one_time_product.purchased" => "purchase.one_time".to_string(),
            "one_time_product.canceled" => "purchase.one_time_cancelled".to_string(),
            "purchase.voided" => "payment.refunded".to_string(),
            "subscription.pending_purchase_canceled" => "subscription.pending_purchase_cancelled".to_string(),
            "refund.created" => "payment.refunded".to_string(),
            "dispute.created" => "dispute.created".to_string(),
            "subscription.update" => "subscription.updated".to_string(),
            "subscription.price_changed" => "subscription.price_changed".to_string(),
            "subscription.price_change_updated" => "subscription.price_change_updated".to_string(),
            "subscription.expired_voided" => "subscription.expired_voided".to_string(),
            "subscription.deferred" => "subscription.deferred".to_string(),
            "subscription.pause_scheduled" => "subscription.pause_scheduled".to_string(),
            "subscription.price_step_up_consent_updated" => "subscription.price_step_up".to_string(),
            _ => event_type.to_string(),
        },
        "lemonsqueezy" => match event_type {
            "subscription_created" => "subscription.created".to_string(),
            "subscription_updated" => "subscription.updated".to_string(),
            "subscription_expired" => "subscription.expired".to_string(),
            "subscription_cancelled" => "subscription.cancelled".to_string(),
            "order_created" => "payment.pending".to_string(),
            "order_failed" => "payment.failed".to_string(),
            "order_completed" => "subscription.activated".to_string(),
            "one_time_product_purchased" => "purchase.one_time".to_string(),
            "one_time_product_canceled" => "purchase.one_time_cancelled".to_string(),
            "purchase_voided" => "payment.refunded".to_string(),
            "pending_purchase_canceled" => "subscription.pending_purchase_cancelled".to_string(),
            "refund_created" => "payment.refunded".to_string(),
            "dispute_created" => "dispute.created".to_string(),
            "price_changed" => "subscription.price_changed".to_string(),
            "price_change_updated" => "subscription.price_change_updated".to_string(),
            _ => event_type.to_string(),
        },
        "coinbase" => match event_type {
            "charge:confirmed" => "charge.confirmed".to_string(),
            "charge:failed" => "charge.failed".to_string(),
            _ => event_type.to_string(),
        },
        _ => event_type.to_string(),
    }
}

/// Normalize raw provider status to canonical Bridge status
fn normalize_status(raw_status: Option<&str>) -> String {
    let Some(s) = raw_status else { return "pending".to_string(); };
    match s.trim().to_ascii_lowercase().as_str() {
        "trialing" | "trial" => "trial".to_string(),
        "active" | "paid" | "completed" | "success" => "active".to_string(),
        "past_due" | "grace_period" => "past_due".to_string(),
        "cancelled" | "canceled" => "cancelled".to_string(),
        "expired" => "expired".to_string(),
        "on_hold" | "on-hold" => "on_hold".to_string(),
        "paused" => "paused".to_string(),
        "revoked" => "revoked".to_string(),
        "pending" => "pending".to_string(),
        _ => s.to_string(),
    }
}

/// Map normalized status to canonical callback event type
fn status_to_canonical_event(normalized_status: &str) -> Option<String> {
    match normalized_status {
        "active" | "trial" => Some("subscription.activated".to_string()),
        "past_due" => Some("subscription.grace_period".to_string()),
        "on_hold" => Some("subscription.on_hold".to_string()),
        "paused" => Some("subscription.paused".to_string()),
        "expired" => Some("subscription.expired".to_string()),
        "cancelled" => Some("subscription.cancelled".to_string()),
        "revoked" => Some("subscription.revoked".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_google_play_events() {
        assert_eq!(
            normalize_event_type("google_play", "SUBSCRIPTION_PURCHASED"),
            "subscription.activated"
        );
        assert_eq!(
            normalize_event_type("google_play", "SUBSCRIPTION_RENEWED"),
            "subscription.renewed"
        );
        assert_eq!(
            normalize_event_type("google_play", "SUBSCRIPTION_CANCELED"),
            "subscription.cancelled"
        );
        assert_eq!(
            normalize_event_type("google_play", "SUBSCRIPTION_ON_HOLD"),
            "subscription.on_hold"
        );
    }

    #[test]
    fn test_normalize_creem_events() {
        assert_eq!(
            normalize_event_type("creem", "subscription.created"),
            "subscription.created"
        );
        assert_eq!(
            normalize_event_type("creem", "subscription.active"),
            "subscription.activated"
        );
        assert_eq!(
            normalize_event_type("creem", "subscription.trialing"),
            "subscription.trial_started"
        );
        assert_eq!(
            normalize_event_type("creem", "subscription.cancelled"),
            "subscription.cancelled"
        );
        assert_eq!(
            normalize_event_type("creem", "refund.created"),
            "payment.refunded"
        );
    }

    #[test]
    fn test_normalize_lemonsqueezy_and_coinbase_special_events() {
        assert_eq!(
            normalize_event_type("lemonsqueezy", "refund_created"),
            "payment.refunded"
        );
        assert_eq!(
            normalize_event_type("coinbase", "charge:failed"),
            "charge.failed"
        );
    }

    #[test]
    fn test_normalize_status() {
        assert_eq!(normalize_status(Some("Trialing")), "trial");
        assert_eq!(normalize_status(Some(" PAID ")), "active");
        assert_eq!(normalize_status(Some("canceled")), "cancelled");
        assert_eq!(normalize_status(None), "pending");
    }

    #[test]
    fn test_status_to_canonical_event() {
        assert_eq!(status_to_canonical_event("active"), Some("subscription.activated".to_string()));
        assert_eq!(status_to_canonical_event("trial"), Some("subscription.activated".to_string()));
        assert_eq!(status_to_canonical_event("expired"), Some("subscription.expired".to_string()));
        assert_eq!(status_to_canonical_event("unknown"), None);
    }

    #[test]
    fn test_extract_metadata_user_id_supports_nested_paths() {
        let payload = serde_json::json!({
            "event": {
                "data": {
                    "metadata": {
                        "external_user_id": "coinbase-user"
                    }
                }
            }
        });

        assert_eq!(extract_metadata_user_id(&payload).as_deref(), Some("coinbase-user"));
    }
}
