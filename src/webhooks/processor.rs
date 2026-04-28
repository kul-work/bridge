use crate::{
    application::app_context::AppSnapshot,
    error::BridgeError,
    ports::{
        WebhookProcessingRepository, WebhookProviderSnapshot, WebhookSubscriptionSnapshot,
    },
};
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
}

async fn suppress_unresolved_webhook<R: WebhookProcessingRepository>(
    repo: &R,
    webhook_provider_id: Uuid,
    webhook: &WebhookProviderSnapshot,
) -> Result<(), BridgeError> {
    warn!(
        "Webhook {} discarded: unable to resolve external_user_id (provider={}, event={})",
        webhook.provider_webhook_id,
        webhook.provider,
        webhook.event_type
    );
    repo.suppress_webhook(webhook_provider_id, "unresolved_external_user_id").await
}

async fn ensure_resolved_user<R: WebhookProcessingRepository>(
    repo: &R,
    webhook_provider_id: Uuid,
    webhook: &WebhookProviderSnapshot,
    external_user_id: &Option<String>,
) -> Result<bool, BridgeError> {
    if external_user_id.is_some() {
        return Ok(true);
    }

    suppress_unresolved_webhook(repo, webhook_provider_id, webhook).await?;
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

async fn enrich_google_play_fields<R: WebhookProcessingRepository>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProviderSnapshot,
    mut fields: WebhookFields,
) -> Result<WebhookFields, BridgeError> {
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
    if std::env::var("MOCK_EXTERNAL_APIS").as_deref() == Ok("true") {
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
) -> Option<String> {
    // Google Play subscription_id is a shared product id, so prefer the
    // purchase token which uniquely identifies the user's subscription.
    if webhook.provider == "google_play" {
        if let Some(ref token) = webhook.purchase_token {
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token(app_id, &webhook.provider, token).await {
                return Some(user);
            }
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token_payment(app_id, &webhook.provider, token).await {
                return Some(user);
            }
        }
    }

    // 1. subscription_id lookup
    if webhook.provider != "google_play" {
        if let Some(ref sub_id) = webhook.subscription_id {
            if let Ok(Some(user)) = repo.lookup_user_by_subscription_id(app_id, &webhook.provider, sub_id).await {
                return Some(user);
            }
        }
    }

    // 2. purchase_token lookup (subscriptions first, then payments)
    if webhook.provider != "google_play" {
        if let Some(ref token) = webhook.purchase_token {
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token(app_id, &webhook.provider, token).await {
                return Some(user);
            }
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token_payment(app_id, &webhook.provider, token).await {
                return Some(user);
            }
        }
    }

    // 3. Google Play Strategy 3 (obfuscated_account_id lookup)
    if webhook.provider == "google_play" {
        // Skip real Google API call in mock mode
        if std::env::var("MOCK_EXTERNAL_APIS").as_deref() == Ok("true") {
            info!("MOCK_EXTERNAL_APIS: Skipping Google Play obfuscated_account_id lookup in resolve_user");
        } else if let Some(ref token) = webhook.purchase_token {
            if let Ok(config) = repo.get_provider_config(app_id, "google_play").await {
                let pkg = config.config.get("package_name").and_then(|v| v.as_str()).unwrap_or("");
                let sa = config.config.get("service_account_json").and_then(|v| v.as_str()).unwrap_or("");
                
                if let Ok(gp_client) = crate::services::google_play::client::GooglePlayClient::new(sa) {
                    if let Ok(sub) = gp_client.get_subscription(pkg, "", token).await {
                        if let Some(ref ids) = sub.external_account_identifiers {
                            if let Some(ref obf_id) = ids.obfuscated_account_id {
                                if let Ok(Some(user)) = repo.lookup_user_by_google_obfuscated_id(app_id, obf_id).await {
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

    let external_user_id = resolve_user(repo, app_id, &webhook).await;
    if !ensure_resolved_user(repo, webhook_provider_id, &webhook, &external_user_id).await? {
        return Ok(None);
    }

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

    let external_user_id = resolve_user(repo, app_id, &webhook).await;
    if !ensure_resolved_user(repo, webhook_provider_id, &webhook, &external_user_id).await? {
        return Ok(None);
    }

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
    };

    // Step 6: Mark webhook as processed
    repo.mark_webhook_processed(webhook_provider_id).await?;

    Ok(Some(canonical))
}


#[cfg(test)]
mod tests;
