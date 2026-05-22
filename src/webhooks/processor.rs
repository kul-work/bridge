use crate::{
    application::app_context::AppSnapshot,
    error::BridgeError,
    ports::{
        WebhookProcessingRepository, WebhookProviderSnapshot, WebhookSubscriptionSnapshot,
    },
    services::google_play::trace::BpTrace,
};
use serde_json::json;
use uuid::Uuid;
use tracing::{error, info, warn};

mod event_handlers;
mod fields;
mod normalize;

use self::{
    event_handlers::{EventContext, EventEffects, EventHandling},
    fields::{extract_metadata_user_id, extract_webhook_fields},
    normalize::{
        callback_status_for_event, normalize_event_type_with_payload,
        normalize_status, parse_rfc3339_utc, status_to_canonical_event,
    },
};
pub(crate) use self::fields::WebhookFields;

struct UserResolution {
    external_user_id: Option<String>,
    failure_summary: Option<String>,
}

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
    pub new_price_cents: Option<i32>,
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
    pub google_price_step_up_consent_deadline: Option<i64>,
    pub google_pause_scheduled_at: Option<i64>,
    pub google_deferred_until: Option<i64>,
}

fn to_epoch_ms(value: Option<chrono::DateTime<chrono::Utc>>) -> Option<i64> {
    value.map(|date| date.timestamp_millis())
}

async fn suppress_unresolved_webhook<R: WebhookProcessingRepository>(
    repo: &R,
    webhook_provider_id: Uuid,
    webhook: &WebhookProviderSnapshot,
    failure_summary: Option<&str>,
) -> Result<(), BridgeError> {
    warn!(
        "Webhook {} discarded: unable to resolve external_user_id (provider={}, event={}, reason={})",
        webhook.provider_webhook_id,
        webhook.provider,
        webhook.event_type,
        failure_summary.unwrap_or("unknown")
    );
    repo.suppress_webhook(webhook_provider_id, "unresolved_external_user_id").await
}

async fn ensure_resolved_user<R: WebhookProcessingRepository>(
    repo: &R,
    webhook_provider_id: Uuid,
    webhook: &WebhookProviderSnapshot,
    resolution: &UserResolution,
) -> Result<bool, BridgeError> {
    if resolution.external_user_id.is_some() {
        return Ok(true);
    }

    suppress_unresolved_webhook(
        repo,
        webhook_provider_id,
        webhook,
        resolution.failure_summary.as_deref(),
    )
    .await?;
    Ok(false)
}

fn extract_customer_email_for_dispute(webhook: &WebhookProviderSnapshot) -> Option<String> {
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
    app: &AppSnapshot,
    webhook: &WebhookProviderSnapshot,
    fields: &WebhookFields,
    external_user_id: Option<&str>,
) -> Result<(), BridgeError> {
    let admin_email = match std::env::var("ADMIN_ALERT_EMAIL")
        .or_else(|_| std::env::var("TYDE_SUPPORT_EMAIL"))
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

    crate::services::email::send_email(&admin_email, &subject, &body)
        .await
        .map_err(|e| BridgeError::InternalServerError(format!("Failed to send dispute admin alert email: {}", e)))?;

    info!(
        "Dispute admin email sent for app_id={} provider_event_id={}",
        app.id,
        webhook.provider_webhook_id
    );

    Ok(())
}

fn mock_google_play_renewal_period_end(
    current_period_end: Option<chrono::DateTime<chrono::Utc>>,
) -> chrono::DateTime<chrono::Utc> {
    current_period_end.unwrap_or_else(chrono::Utc::now) + chrono::Duration::days(30)
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

fn google_subscription_expiry_time(
    resource: &crate::services::google_play::models::SubscriptionPurchaseV2,
) -> Option<String> {
    resource
        .line_items
        .first()
        .and_then(|line_item| line_item.expiry_time.clone())
        .or_else(|| resource.expiry_time.clone())
}

fn google_subscription_transaction_id(
    resource: &crate::services::google_play::models::SubscriptionPurchaseV2,
    provider_webhook_id: &str,
) -> String {
    resource
        .latest_order_id
        .clone()
        .unwrap_or_else(|| format!("google_play_rtdn:{}", provider_webhook_id))
}

fn google_money_to_cents(money: &crate::services::google_play::models::Money) -> Option<i32> {
    let units = money.units.as_deref().unwrap_or("0").parse::<i64>().ok()?;
    let nanos = i64::from(money.nanos.unwrap_or(0));
    let cents = units.checked_mul(100)?.checked_add(nanos / 10_000_000)?;
    i32::try_from(cents).ok()
}

fn google_subscription_recurring_amount_cents(
    resource: &crate::services::google_play::models::SubscriptionPurchaseV2,
) -> Option<i32> {
    resource
        .line_items
        .first()
        .and_then(|line_item| line_item.auto_renewing_plan.as_ref())
        .and_then(|plan| plan.recurring_price.as_ref())
        .and_then(google_money_to_cents)
}

fn google_subscription_recurring_currency(
    resource: &crate::services::google_play::models::SubscriptionPurchaseV2,
) -> Option<String> {
    resource
        .line_items
        .first()
        .and_then(|line_item| line_item.auto_renewing_plan.as_ref())
        .and_then(|plan| plan.recurring_price.as_ref())
        .and_then(|money| money.currency_code.clone())
}

fn google_voided_purchase_product_type(payload: &serde_json::Value) -> Option<i64> {
    payload
        .pointer("/voidedPurchaseNotification/productType")
        .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok())))
}

fn is_google_play_one_time_webhook(webhook: &WebhookProviderSnapshot) -> bool {
    if webhook.provider != "google_play" {
        return false;
    }

    webhook.event_type.starts_with("ONE_TIME_PRODUCT_")
        || webhook.payload.get("oneTimeProductNotification").is_some()
        || google_voided_purchase_product_type(&webhook.payload) == Some(2)
}

async fn enrich_google_play_fields<R: WebhookProcessingRepository>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProviderSnapshot,
    mut fields: WebhookFields,
) -> Result<WebhookFields, BridgeError> {
    if is_google_play_one_time_webhook(webhook) {
        return Ok(fields);
    }

    let Some(purchase_token) = fields.purchase_token.as_deref().or(webhook.purchase_token.as_deref()) else {
        return Ok(fields);
    };

    let Some(subscription_id) = fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()) else {
        return Ok(fields);
    };

    let provider_config = match repo.get_provider_config(app_id, "google_play").await {
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

    // Skip API call in mock mode
    if crate::config::mock_external_apis_enabled() {
        tracing::info!("MOCK_EXTERNAL_APIS: Skipping Google Play API enrichment in webhook processing");

        if webhook.event_type == "SUBSCRIPTION_RENEWED" && fields.current_period_end.is_none() {
            if let Ok(Some(subscription)) = repo.get_subscription_by_purchase_token_for_provider(app_id, &webhook.provider, purchase_token).await {
                // Mirror the mock verify-purchase flow: renewals advance the existing mock period by 30 days.
                fields.current_period_end = Some(
                    mock_google_play_renewal_period_end(subscription.current_period_end).to_rfc3339(),
                );
            }
        }

        // SUBSCRIPTION_RESTARTED: user re-enabled auto-renew after cancellation or un-paused.
        // The real Google Play API would return auto_renewing=true and a future expiry.
        if webhook.event_type == "SUBSCRIPTION_RESTARTED" {
            fields.auto_renewing = Some(true);
            if fields.current_period_end.is_none() {
                if let Ok(Some(subscription)) = repo.get_subscription_by_purchase_token_for_provider(app_id, &webhook.provider, purchase_token).await {
                    fields.current_period_end = Some(
                        mock_google_play_renewal_period_end(subscription.current_period_end).to_rfc3339(),
                    );
                }
            }
        }

        // Allow test harness to override the price via X-Test-Price-Cents header
        // (injected into payload as _test_price_cents by ingress)
        if let Some(test_price) = webhook.payload.get("_test_price_cents").and_then(|v| v.as_i64()) {
            fields.amount_cents = Some(test_price as i32);
        }

        if webhook.event_type.starts_with("SUBSCRIPTION_") && fields.provider_transaction_id.is_none() {
            fields.provider_transaction_id = Some(format!("google_play_rtdn:{}", webhook.provider_webhook_id));
        }

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
        fields.current_period_end = google_subscription_expiry_time(&resource);
    }

    if webhook.event_type.starts_with("SUBSCRIPTION_") {
        fields.provider_transaction_id = Some(google_subscription_transaction_id(
            &resource,
            &webhook.provider_webhook_id,
        ));
    }

    if fields.amount_cents.is_none() {
        fields.amount_cents = google_subscription_recurring_amount_cents(&resource);
    }

    if fields.currency.is_none() {
        fields.currency = google_subscription_recurring_currency(&resource);
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

async fn fill_payment_product_id<R: WebhookProcessingRepository>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProviderSnapshot,
    fields: &mut WebhookFields,
) -> Result<(), BridgeError> {
    if fields.product_id.is_some() {
        return Ok(());
    }

    let Some(purchase_token) = fields.purchase_token.as_deref().or(webhook.purchase_token.as_deref()) else {
        return Ok(());
    };

    fields.product_id = repo
        .lookup_product_id_by_purchase_token_payment(app_id, &webhook.provider, purchase_token)
        .await?;

    Ok(())
}

/// Resolve external_user_id via lookup cascade.
async fn resolve_user<R: WebhookProcessingRepository>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProviderSnapshot,
) -> UserResolution {
    let mut failure_parts: Vec<&'static str> = Vec::new();

    // Google Play subscription_id is a shared product id, so prefer the
    // purchase token which uniquely identifies the user's subscription.
    if webhook.provider == "google_play" {
        if let Some(ref token) = webhook.purchase_token {
            match repo.lookup_user_by_purchase_token(app_id, &webhook.provider, token).await {
                Ok(Some(user)) => return UserResolution { external_user_id: Some(user), failure_summary: None },
                Ok(None) => failure_parts.push("subscription_token=miss"),
                Err(_) => failure_parts.push("subscription_token=error"),
            }
            match repo.lookup_user_by_purchase_token_payment(app_id, &webhook.provider, token).await {
                Ok(Some(user)) => return UserResolution { external_user_id: Some(user), failure_summary: None },
                Ok(None) => failure_parts.push("payment_token=miss"),
                Err(_) => failure_parts.push("payment_token=error"),
            }
        } else {
            failure_parts.push("subscription_token=missing");
            failure_parts.push("payment_token=missing");
        }
    }

    // 1. subscription_id lookup
    if webhook.provider != "google_play" {
        if let Some(ref sub_id) = webhook.subscription_id {
            if let Ok(Some(user)) = repo.lookup_user_by_subscription_id(app_id, &webhook.provider, sub_id).await {
                return UserResolution { external_user_id: Some(user), failure_summary: None };
            }
        }
    }

    // 2. purchase_token lookup (subscriptions first, then payments)
    if webhook.provider != "google_play" {
        if let Some(ref token) = webhook.purchase_token {
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token(app_id, &webhook.provider, token).await {
                return UserResolution { external_user_id: Some(user), failure_summary: None };
            }
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token_payment(app_id, &webhook.provider, token).await {
                return UserResolution { external_user_id: Some(user), failure_summary: None };
            }
        }
    }

    // 3. Google Play Strategy 3 (obfuscated_account_id lookup)
    if webhook.provider == "google_play" {
        // Skip real Google API call in mock mode
        if crate::config::mock_external_apis_enabled() {
            failure_parts.push("obfuscated_account_id=skipped_mock");
        } else if is_google_play_one_time_webhook(webhook) {
            failure_parts.push("obfuscated_account_id=skipped_otp");
        } else if let Some(ref token) = webhook.purchase_token {
            if let Ok(config) = repo.get_provider_config(app_id, "google_play").await {
                let pkg = config.config.get("package_name").and_then(|v| v.as_str()).unwrap_or("");
                let sa = config.config.get("service_account_json").and_then(|v| v.as_str()).unwrap_or("");
                
                if let Ok(gp_client) = crate::services::google_play::client::GooglePlayClient::new(sa) {
                    if let Ok(sub) = gp_client.get_subscription(pkg, "", token).await {
                        if let Some(ref ids) = sub.external_account_identifiers {
                            if let Some(ref obf_id) = ids.obfuscated_account_id {
                                if let Ok(Some(user)) = repo.lookup_user_by_google_obfuscated_id(app_id, obf_id).await {
                                    return UserResolution { external_user_id: Some(user), failure_summary: None };
                                }
                                failure_parts.push("obfuscated_account_id=miss");
                            } else {
                                failure_parts.push("obfuscated_account_id=missing");
                            }
                        } else {
                            failure_parts.push("obfuscated_account_id=missing");
                        }
                    } else {
                        failure_parts.push("obfuscated_account_id=api_error");
                    }
                } else {
                    failure_parts.push("obfuscated_account_id=client_error");
                }
            } else {
                failure_parts.push("obfuscated_account_id=config_error");
            }
        }
    }

    // 4. Provider metadata.user_id / external_user_id
    if let Some(user_id) = extract_metadata_user_id(&webhook.payload) {
        return UserResolution { external_user_id: Some(user_id), failure_summary: None };
    }
    failure_parts.push("metadata=missing");

    // 5. Creem orphan guard
    if webhook.provider == "creem" {
        error!(
            "Creem orphan guard: discarding webhook {} (event={})",
            webhook.provider_webhook_id, webhook.event_type
        );
        return UserResolution {
            external_user_id: None,
            failure_summary: Some(failure_parts.join(", ")),
        };
    }

    // 6. All strategies failed
    UserResolution {
        external_user_id: None,
        failure_summary: Some(failure_parts.join(", ")),
    }
}

pub async fn build_canonical_payload<R: WebhookProcessingRepository>(
    repo: &R,
    webhook_provider_id: Uuid,
    app_id: Uuid,
) -> Result<Option<CanonicalWebhookPayload>, BridgeError> {
    let webhook = repo.get_webhook_provider(webhook_provider_id).await?;

    if webhook.suppressed {
        info!(
            "Webhook {} already suppressed: {}",
            webhook_provider_id, webhook.suppressed_reason.as_deref().unwrap_or("unknown")
        );
        return Ok(None);
    }

    let app = repo
        .get_app(app_id)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let resolution = resolve_user(repo, app_id, &webhook).await;
    if !ensure_resolved_user(repo, webhook_provider_id, &webhook, &resolution).await? {
        return Ok(None);
    }
    let external_user_id = resolution.external_user_id;

    let mut fields = extract_webhook_fields(&webhook);
    if webhook.provider == "google_play" {
        fields = enrich_google_play_fields(repo, app_id, &webhook, fields).await?;
    }
    fill_payment_product_id(repo, app_id, &webhook, &mut fields).await?;
    let canonical_event = normalize_event_type_with_payload(
        &webhook.provider,
        &webhook.event_type,
        Some(&webhook.payload),
    );
    let timestamp_epoch_ms = webhook
        .timestamp_epoch_ms
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let timestamp_iso = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();
    let canonical_subscription = if let Some(ref sub_id) = fields.subscription_id {
        repo.get_subscription_by_sub_id_for_provider(app_id, &webhook.provider, sub_id).await?
    } else if let Some(ref token) = fields.purchase_token {
        repo.get_subscription_by_purchase_token_for_provider(app_id, &webhook.provider, token).await?
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
        "purchase.one_time_refunded" => "purchase.one_time".to_string(),
        "payment.refunded" => "payment.refunded".to_string(),
        "payment.partially_refunded" => "payment.partially_refunded".to_string(),
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
        "payment.partially_refunded" => Some("partially_refunded".to_string()),
        "purchase.one_time" => Some(if canonical_event == "purchase.one_time_cancelled" {
            "cancelled".to_string()
        } else if canonical_event == "purchase.one_time_refunded" {
            "refunded".to_string()
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
            .or_else(|| callback_status_for_event(&callback_event_type)),
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
    let google_price_step_up_consent_deadline = canonical_subscription
        .as_ref()
        .and_then(|sub| to_epoch_ms(sub.google_price_step_up_consent_deadline));
    let google_pause_scheduled_at = canonical_subscription
        .as_ref()
        .and_then(|sub| to_epoch_ms(sub.google_pause_scheduled_at));
    let google_deferred_until = canonical_subscription
        .as_ref()
        .and_then(|sub| to_epoch_ms(sub.google_deferred_until));

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
        new_price_cents: fields.google_new_price_cents,
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
        google_price_step_up_consent_deadline,
        google_pause_scheduled_at,
        google_deferred_until,
    }))
}

fn apply_event_effects(
    effects: EventEffects,
    callback_event_type: &mut String,
    callback_status_override: &mut Option<String>,
    callback_revocation_reason_override: &mut Option<String>,
    callback_cancellation_mode_override: &mut Option<String>,
    canonical_subscription: &mut Option<WebhookSubscriptionSnapshot>,
    should_forward: &mut bool,
) {
    if let Some(event_type) = effects.callback_event_type {
        *callback_event_type = event_type;
    }

    *callback_status_override = effects.callback_status_override;
    *callback_revocation_reason_override = effects.callback_revocation_reason_override;
    *callback_cancellation_mode_override = effects.callback_cancellation_mode_override;
    *canonical_subscription = effects.canonical_subscription;
    *should_forward = effects.should_forward;
}

fn google_play_notification_type(payload: &serde_json::Value) -> Option<i64> {
    payload
        .pointer("/subscriptionNotification/notificationType")
        .or_else(|| payload.pointer("/oneTimeProductNotification/notificationType"))
        .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok())))
}

fn json_string_at<'a>(payload: &'a serde_json::Value, pointer: &str) -> Option<&'a str> {
    payload.pointer(pointer).and_then(|value| value.as_str())
}

fn json_i64_at(payload: &serde_json::Value, pointer: &str) -> Option<i64> {
    payload
        .pointer(pointer)
        .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok())))
}

fn emit_webhook_trace(
    app_id: Uuid,
    canonical: &CanonicalWebhookPayload,
    provider_payload: &serde_json::Value,
) {
    let mut trace = BpTrace::new("webhook", &canonical.provider_event_id);

    if let Some(external_user_id) = canonical.external_user_id.as_deref() {
        trace.set_user_id(external_user_id);
    }
    if let Some(subscription_id) = canonical.subscription_id.as_deref() {
        trace.set_subscription_id(subscription_id);
    }
    if let Some(purchase_token) = canonical.purchase_token.as_deref() {
        trace.set_token_hash(purchase_token);
        trace.add_metadata("hash_source", json!("purchase_token"));
    } else {
        trace.set_token_hash(&canonical.provider_event_id);
        trace.add_metadata("hash_source", json!("provider_event_id"));
    }
    if canonical.provider == "google_play" {
        if let Some(package_name) = json_string_at(provider_payload, "/packageName") {
            trace.add_metadata("packageName", json!(package_name));
        }
        if let Some(event_time_ms) = json_i64_at(provider_payload, "/eventTimeMillis") {
            trace.add_metadata("eventTimeMillis", json!(event_time_ms));
        }
        if let Some(notification_type) = google_play_notification_type(provider_payload) {
            trace.add_metadata("notificationType", json!(notification_type));
        }
        if let Some(sku) = json_string_at(provider_payload, "/oneTimeProductNotification/sku")
            .or_else(|| json_string_at(provider_payload, "/oneTimeProductNotification/productId"))
        {
            trace.add_metadata("sku", json!(sku));
        }
        if let Some(order_id) = json_string_at(provider_payload, "/voidedPurchaseNotification/orderId") {
            trace.add_metadata("orderId", json!(order_id));
        }
        if let Some(product_type) = json_i64_at(provider_payload, "/voidedPurchaseNotification/productType") {
            trace.add_metadata("productType", json!(product_type));
        }
        if let Some(refund_type) = json_i64_at(provider_payload, "/voidedPurchaseNotification/refundType") {
            trace.add_metadata("refundType", json!(refund_type));
        }
    }

    trace
        .set_step("finish")
        .set_result("success")
        .add_metadata("app_id", json!(app_id.to_string()))
        .add_metadata("app_slug", json!(canonical.app_slug.as_str()))
        .add_metadata("provider", json!(canonical.provider.as_str()))
        .add_metadata("event_type", json!(canonical.event_type.as_str()))
        .add_metadata("status", json!(canonical.status.as_deref()))
        .add_metadata("current_period_end", json!(canonical.current_period_end.as_deref()))
        .add_metadata("auto_renewing", json!(canonical.auto_renewing))
        .add_metadata("amount_cents", json!(canonical.amount_cents))
        .emit();
}

/// Process webhook: dedup, ordering, normalization, DB mutations
pub async fn process_webhook(
    repo: &impl WebhookProcessingRepository,
    webhook_provider_id: Uuid,
    app_id: Uuid,
) -> Result<Option<CanonicalWebhookPayload>, BridgeError> {
    // Step 1: Load webhook + app
    let webhook = repo.get_webhook_provider(webhook_provider_id).await?;

    if webhook.suppressed {
        info!(
            "Webhook {} already suppressed: {}",
            webhook_provider_id, webhook.suppressed_reason.as_deref().unwrap_or("unknown")
        );
        return Ok(None);
    }

    let app = repo
        .get_app(app_id)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let canonical_event = normalize_event_type_with_payload(
        &webhook.provider,
        &webhook.event_type,
        Some(&webhook.payload),
    );

    let resolution = resolve_user(repo, app_id, &webhook).await;
    if !ensure_resolved_user(repo, webhook_provider_id, &webhook, &resolution).await? {
        return Ok(None);
    }
    let external_user_id = resolution.external_user_id;

    let mut fields = extract_webhook_fields(&webhook);
    if webhook.provider == "google_play" {
        fields = enrich_google_play_fields(repo, app_id, &webhook, fields).await?;
    }
    fill_payment_product_id(repo, app_id, &webhook, &mut fields).await?;
    let provider = webhook.provider.clone();
    let webhook_provider_webhook_id = webhook.provider_webhook_id.clone();
    let webhook_subscription_id = webhook.subscription_id.clone();
    let webhook_purchase_token = webhook.purchase_token.clone();
    let fields_subscription_id = fields.subscription_id.clone();
    let fields_purchase_token = fields.purchase_token.clone();
    let fields_current_period_end = fields.current_period_end.clone();
    let fields_status = fields.status.clone();

    let timestamp_epoch_ms = webhook.timestamp_epoch_ms.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let timestamp_iso = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();
    let mut callback_event_type = canonical_event.clone();
    let mut callback_status_override: Option<String> = None;
    let mut callback_revocation_reason_override: Option<String> = None;
    let mut callback_cancellation_mode_override: Option<String> = None;
    let mut canonical_subscription: Option<WebhookSubscriptionSnapshot> = None;
    let mut should_forward = true;
    let event_context = EventContext {
        app: &app,
        app_id,
        canonical_event: &canonical_event,
        provider: &provider,
        webhook: &webhook,
        fields: &fields,
        external_user_id: &external_user_id,
        timestamp_epoch_ms,
    };

    // Step 4: Route by canonical event type and mutate DB
    let handling = match event_handlers::handle_subscription_event(repo, &event_context).await? {
        EventHandling::NotHandled => match event_handlers::handle_payment_event(repo, &event_context).await? {
            EventHandling::NotHandled => event_handlers::handle_provider_event(repo, &event_context).await?,
            handled => handled,
        },
        handled => handled,
    };

    match handling {
        EventHandling::Handled(effects) => apply_event_effects(
            effects,
            &mut callback_event_type,
            &mut callback_status_override,
            &mut callback_revocation_reason_override,
            &mut callback_cancellation_mode_override,
            &mut canonical_subscription,
            &mut should_forward,
        ),
        EventHandling::ReturnNone => return Ok(None),
        EventHandling::NotHandled => {
            info!("Unhandled webhook event type: {} (provider: {})", canonical_event, webhook.provider);
        }
    }

    if !should_forward {
        repo.mark_webhook_processed(webhook_provider_id).await?;
        return Ok(None);
    }

    // Step 5: Build canonical payload with real data
    let canonical_status = callback_status_override
        .or_else(|| canonical_subscription.as_ref().map(|sub| sub.status.clone()))
        .or_else(|| fields_status.clone())
        .or_else(|| callback_status_for_event(&callback_event_type));
    let canonical_current_period_end = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.current_period_end.map(|value| value.to_rfc3339()))
        .or_else(|| fields_current_period_end.clone());
    let canonical_auto_renewing = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.auto_renewing)
        .or(fields.auto_renewing);
    let canonical_purchase_token = canonical_subscription
        .as_ref()
        .and_then(|sub| sub.purchase_token.clone())
        .or_else(|| fields_purchase_token.clone())
        .or_else(|| webhook_purchase_token.clone());
    let canonical_revocation_reason = callback_revocation_reason_override
        .or_else(|| canonical_subscription.as_ref().and_then(|sub| sub.revocation_reason.clone()))
        .or_else(|| fields.cancel_reason.clone());
    let google_price_step_up_consent_deadline = canonical_subscription
        .as_ref()
        .and_then(|sub| to_epoch_ms(sub.google_price_step_up_consent_deadline));
    let google_pause_scheduled_at = canonical_subscription
        .as_ref()
        .and_then(|sub| to_epoch_ms(sub.google_pause_scheduled_at));
    let google_deferred_until = canonical_subscription
        .as_ref()
        .and_then(|sub| to_epoch_ms(sub.google_deferred_until));
    let canonical = CanonicalWebhookPayload {
        event_id: format!("{}-{}", provider, webhook_provider_webhook_id),
        event_type: callback_event_type,
        timestamp: timestamp_iso,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: fields.product_id,
        subscription_id: fields_subscription_id.clone().or(webhook_subscription_id.clone()),
        external_user_id,
        amount_cents: fields.amount_cents,
        new_price_cents: fields.google_new_price_cents,
        auto_renewing: canonical_auto_renewing,
        purchase_token: canonical_purchase_token,
        current_period_end: canonical_current_period_end,
        status: canonical_status,
        provider: provider.clone(),
        provider_event_id: webhook_provider_webhook_id.clone(),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: canonical_revocation_reason,
        cancellation_mode: callback_cancellation_mode_override,
        google_price_step_up_consent_deadline,
        google_pause_scheduled_at,
        google_deferred_until,
    };

    // Step 6: Mark webhook as processed
    repo.mark_webhook_processed(webhook_provider_id).await?;

    emit_webhook_trace(app_id, &canonical, &webhook.payload);

    Ok(Some(canonical))
}


#[cfg(test)]
mod tests;
