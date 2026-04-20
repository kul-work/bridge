use crate::{
    application::app_context::AppSnapshot,
    error::BridgeError,
    ports::{
        SubscriptionWebhookTransition, WebhookPaymentRecordRequest,
        WebhookProcessingRepository, WebhookProviderSnapshot,
        WebhookSubscriptionCommitRequest, WebhookSubscriptionSnapshot,
    },
};
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

fn normalize_event_type_with_payload(
    provider: &str,
    event_type: &str,
    payload: Option<&serde_json::Value>,
) -> String {
    if provider == "creem" && event_type == "checkout.completed" {
        if let Some(payload) = payload {
            let object = payload.get("object").unwrap_or(&serde_json::Value::Null);
            let billing_type = object.get("billing_type")
                .and_then(|v| v.as_str())
                .or_else(|| object.get("product")
                    .and_then(|v| v.get("billing_type"))
                    .and_then(|v| v.as_str()))
                .or_else(|| object.get("order")
                    .and_then(|v| v.get("type"))
                    .and_then(|v| v.as_str()))
                .or_else(|| object.get("subscription")
                    .and_then(|v| v.get("product"))
                    .and_then(|v| v.get("billing_type"))
                    .and_then(|v| v.as_str()));

            if matches!(billing_type, Some("recurring") | Some("monthly")) {
                return "subscription.created".to_string();
            }

            if matches!(billing_type, Some("one_time") | Some("one-time") | Some("otp") | Some("lifetime")) {
                return "purchase.one_time".to_string();
            }

            if object.pointer("/subscription/id").and_then(|v| v.as_str()).is_some() {
                return "subscription.created".to_string();
            }

            if object.get("checkout_id").and_then(|v| v.as_str()).is_some()
                || object.get("order_id").and_then(|v| v.as_str()).is_some()
                || object.pointer("/order/id").and_then(|v| v.as_str()).is_some()
            {
                return "purchase.one_time".to_string();
            }
        }
    }

    normalize_event_type(provider, event_type)
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

#[allow(dead_code)]
pub(crate) struct WebhookFields {
    pub(crate) subscription_id: Option<String>,
    pub(crate) purchase_token: Option<String>,
    pub(crate) amount_cents: Option<i32>,
    pub(crate) auto_renewing: Option<bool>,
    pub(crate) current_period_end: Option<String>,
    pub(crate) provider_transaction_id: Option<String>,
    pub(crate) provider_customer_id: Option<String>,
    pub(crate) product_id: Option<String>,
    pub(crate) cancel_reason: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) google_subscription_state: Option<i32>,
    pub(crate) google_cancellation_context: Option<String>,
    pub(crate) google_cancellation_feedback: Option<String>,
    pub(crate) google_new_price_cents: Option<i32>,
    pub(crate) google_price_step_up_consent_deadline: Option<String>,
}

fn extract_metadata_user_id(payload: &serde_json::Value) -> Option<String> {
    [
        "/metadata/user_id",
        "/object/metadata/user_id",
        "/object/checkout/metadata/user_id",
        "/event/data/metadata/external_user_id",
        "/event/data/metadata/user_id",
    ]
    .into_iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(|value| value.as_str()).map(|value| value.to_string()))
}

async fn suppress_unresolved_webhook<R: WebhookProcessingRepository>(
    repo: &R,
    webhook_provider_id: Uuid,
    webhook: &WebhookProviderSnapshot,
) -> Result<(), BridgeError> {
    error!(
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

fn extract_webhook_fields(webhook: &WebhookProviderSnapshot) -> WebhookFields {
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
        "creem" => {
            let obj = p.get("object").unwrap_or(&serde_json::Value::Null);
            let raw_event_type = p.get("eventType").and_then(|v| v.as_str()).unwrap_or("");
            
            // Extract top-level identifiers with fallbacks
            let object_id = obj.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
            let object_subscription_id = obj.get("subscription_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| obj.get("subscription")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()));
            let object_checkout_id = obj.get("checkout_id").and_then(|v| v.as_str()).map(|s| s.to_string());
            let object_order_id = obj.get("order_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| obj.get("order")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()));

            // Extract product_id with fallbacks (direct, nested.id, checkout product)
            let object_product_id = obj.get("product_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| obj.get("product")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()))
                .or_else(|| obj.get("checkout")
                    .and_then(|v| v.get("product"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()));

            let normalized_event_type = normalize_event_type_with_payload(
                "creem",
                raw_event_type,
                Some(p),
            );
            let subscription_obj = if raw_event_type == "checkout.completed"
                && normalized_event_type == "subscription.created"
            {
                obj.get("subscription").unwrap_or(&serde_json::Value::Null).clone()
            } else {
                obj.clone()
            };

            // Determine subscription_id based on event type
            let subscription_id = match normalized_event_type.as_str() {
                "purchase.one_time" => object_product_id.clone().or_else(|| object_id.clone()),
                "payment.refunded" => object_subscription_id.clone()
                    .or_else(|| object_product_id.clone())
                    .or_else(|| object_id.clone()),
                "subscription.created" => {
                    if raw_event_type == "checkout.completed" {
                        subscription_obj.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
                            .or_else(|| object_subscription_id.clone())
                    } else {
                        object_subscription_id.clone().or_else(|| object_id.clone())
                    }
                }
                _ => object_subscription_id.clone().or_else(|| object_id.clone()),
            };

            // Extract status from subscription object (for checkout.completed with recurring, nested in subscription)
            let status = subscription_obj.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());

            // Extract current_period_end with fallback to renews_at
            let current_period_end = subscription_obj.get("current_period_end_date")
                .and_then(|v| v.as_str())
                .or_else(|| subscription_obj.get("renews_at").and_then(|v| v.as_str()))
                .map(|s| s.to_string());

            // Extract amount with multiple fallbacks (last_transaction.amount, order.amount, product.price, amount)
            let amount_cents = obj.get("last_transaction")
                .and_then(|v| v.get("amount"))
                .and_then(|v| v.as_i64())
                .or_else(|| obj.get("order")
                    .and_then(|v| v.get("amount"))
                    .and_then(|v| v.as_i64()))
                .or_else(|| obj.get("product")
                    .and_then(|v| v.get("price"))
                    .and_then(|v| v.as_i64()))
                .or_else(|| obj.get("amount").and_then(|v| v.as_i64()))
                .map(|a| a as i32);

            // Extract purchase_token (checkout_id for OTP, order_id for refunds)
            let purchase_token = match normalized_event_type.as_str() {
                "purchase.one_time" => object_checkout_id
                    .or_else(|| object_order_id.clone())
                    .or_else(|| object_id.clone()),
                "payment.refunded" => object_order_id
                    .or_else(|| object_checkout_id)
                    .or_else(|| object_subscription_id.clone())
                    .or_else(|| object_id.clone()),
                _ => None,
            };

            // Extract provider_transaction_id (last_transaction_id)
            let provider_transaction_id = obj.get("last_transaction_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string());

            WebhookFields {
                subscription_id,
                purchase_token,
                amount_cents,
                auto_renewing: obj.get("auto_renewing").and_then(|v| v.as_bool()),
                current_period_end,
                provider_transaction_id,
                provider_customer_id: obj.get("customer")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                product_id: object_product_id,
                cancel_reason: None,
                status,
                google_subscription_state: None,
                google_cancellation_context: None,
                google_cancellation_feedback: None,
                google_new_price_cents: None,
                google_price_step_up_consent_deadline: None,
            }
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
            if let Ok(Some(subscription)) = repo.get_subscription_by_purchase_token(app_id, purchase_token).await {
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
                if let Ok(Some(subscription)) = repo.get_subscription_by_purchase_token(app_id, purchase_token).await {
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

/// Resolve external_user_id via §53 cascade
async fn resolve_user<R: WebhookProcessingRepository>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProviderSnapshot,
) -> Option<String> {
    // Google Play subscription_id is a shared product id, so prefer the
    // purchase token which uniquely identifies the user's subscription.
    if webhook.provider == "google_play" {
        if let Some(ref token) = webhook.purchase_token {
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token(app_id, token).await {
                return Some(user);
            }
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token_payment(app_id, token).await {
                return Some(user);
            }
        }
    }

    // 1. subscription_id lookup
    if webhook.provider != "google_play" {
        if let Some(ref sub_id) = webhook.subscription_id {
            if let Ok(Some(user)) = repo.lookup_user_by_subscription_id(app_id, sub_id).await {
                return Some(user);
            }
        }
    }

    // 2. purchase_token lookup (subscriptions first, then payments)
    if webhook.provider != "google_play" {
        if let Some(ref token) = webhook.purchase_token {
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token(app_id, token).await {
                return Some(user);
            }
            if let Ok(Some(user)) = repo.lookup_user_by_purchase_token_payment(app_id, token).await {
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
        repo.get_subscription_by_sub_id(app_id, sub_id).await?
    } else if let Some(ref token) = fields.purchase_token {
        repo.get_subscription_by_purchase_token(app_id, token).await?
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

/// Process webhook: dedup, ordering, normalization, DB mutations
#[allow(dead_code)]
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

        repo.mark_webhook_processed(webhook_provider_id).await?;

        return Ok(None);
    }

    let external_user_id = resolve_user(repo, app_id, &webhook).await;
    if !ensure_resolved_user(repo, webhook_provider_id, &webhook, &external_user_id).await? {
        return Ok(None);
    }

    let mut fields = extract_webhook_fields(&webhook);
    if webhook.provider == "google_play" {
        fields = enrich_google_play_fields(repo, app_id, &webhook, fields).await?;
    }
    let provider = webhook.provider.clone();
    let webhook_provider_webhook_id = webhook.provider_webhook_id.clone();
    let webhook_subscription_id = webhook.subscription_id.clone();
    let webhook_purchase_token = webhook.purchase_token.clone();
    let fields_subscription_id = fields.subscription_id.clone();
    let fields_purchase_token = fields.purchase_token.clone();
    let fields_current_period_end = fields.current_period_end.clone();
    let fields_status = fields.status.clone();
    let fields_provider_customer_id = fields.provider_customer_id.clone();

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

    // Step 4: Route by canonical event type and mutate DB
    match canonical_event.as_str() {
        // §13 - Subscription Activation
        "subscription.activated" | "subscription.renewed" | "subscription.recovered" | "subscription.created" => {
            if let Some(ref user_id) = external_user_id {
                let period_end = fields_current_period_end.as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let sub_id_fallback = webhook.subscription_id.clone().unwrap_or_default();
                let sub_id_str = fields_subscription_id.as_deref()
                    .unwrap_or(&sub_id_fallback);

                let subscription = repo
                    .commit_webhook_subscription(WebhookSubscriptionCommitRequest {
                        app_id,
                        external_user_id: user_id,
                        subscription_id: sub_id_str,
                        provider: &provider,
                        status: "active",
                        current_period_end: period_end,
                        purchase_token: fields_purchase_token.as_deref(),
                        auto_renewing: fields.auto_renewing,
                        payment_state: None,
                        provider_customer_id: fields_provider_customer_id.as_deref(),
                        event_time_ms: timestamp_epoch_ms,
                        payment: fields.provider_transaction_id.as_deref().map(|txn_id| {
                            WebhookPaymentRecordRequest {
                                app_id,
                                external_user_id: user_id,
                                provider: &provider,
                                provider_transaction_id: txn_id,
                                subscription_id: fields_subscription_id.as_deref(),
                                amount_cents: fields.amount_cents.unwrap_or(0),
                                status: "success",
                            }
                        }),
                        adopt_stale_payment: provider == "creem",
                    })
                    .await?;

                let Some(subscription) = subscription else {
                    info!(
                        "Skipped stale activation event for subscription {} (provider: {})",
                        sub_id_str,
                        webhook.provider
                    );
                    return Ok(None);
                };

                canonical_subscription = Some(subscription);
                callback_event_type = "subscription.activated".to_string();
                callback_status_override = Some("active".to_string());

                    if provider == "google_play" {
                        let _ = repo.link_replacement_subscriptions(app_id, user_id, sub_id_str, timestamp_epoch_ms).await;
                    }
            }
        }
        
        // §14 - Subscription Pending
        "subscription.pending" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.as_deref()
                    .or(webhook.subscription_id.as_deref())
                    .unwrap_or("");
                let updated = repo.apply_subscription_transition(
                    app_id,
                    sub_id,
                    timestamp_epoch_ms,
                    SubscriptionWebhookTransition::Pending,
                ).await?;
                if updated.is_none() {
                    info!("Skipped stale pending event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
            should_forward = false;
        }

        // §15 - Grace Period
        "subscription.trial_started" => {
            if let Some(ref user_id) = external_user_id {
                let period_end = fields_current_period_end.as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let sub_id_fallback = webhook.subscription_id.clone().unwrap_or_default();
                let sub_id_str = fields_subscription_id.as_deref()
                    .unwrap_or(&sub_id_fallback);

                let subscription = repo
                    .commit_webhook_subscription(WebhookSubscriptionCommitRequest {
                        app_id,
                        external_user_id: user_id,
                        subscription_id: sub_id_str,
                        provider: &provider,
                        status: "trial",
                        current_period_end: period_end,
                        purchase_token: fields_purchase_token.as_deref(),
                        auto_renewing: fields.auto_renewing,
                        payment_state: None,
                        provider_customer_id: fields_provider_customer_id.as_deref(),
                        event_time_ms: timestamp_epoch_ms,
                        payment: None,
                        adopt_stale_payment: false,
                    })
                    .await?;

                let Some(subscription) = subscription else {
                    info!(
                        "Skipped stale trial-start event for subscription {} (provider: {})",
                        sub_id_str,
                        provider
                    );
                    return Ok(None);
                };

                canonical_subscription = Some(subscription);
                callback_event_type = "subscription.activated".to_string();
                callback_status_override = Some("trial".to_string());
            }
        }

        "subscription.grace_period" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                let grace_end = fields
                    .current_period_end
                    .as_deref()
                    .and_then(parse_rfc3339_utc);
                let updated = repo.apply_subscription_transition(
                    app_id,
                    &sub_id,
                    timestamp_epoch_ms,
                    SubscriptionWebhookTransition::GracePeriod {
                        grace_period_end: grace_end,
                    },
                ).await?;
                if let Some(sub) = updated {
                    canonical_subscription = Some(sub.into());
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
            if let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_revoked(
                repo,
                app_id,
                &webhook,
                &fields,
                timestamp_epoch_ms,
            ).await? {
                canonical_subscription = outcome.canonical_subscription;
                if let Some(event_type) = outcome.callback_event_type {
                    callback_event_type = event_type;
                }
                callback_status_override = outcome.callback_status_override;
                callback_revocation_reason_override = outcome.callback_revocation_reason_override;
                callback_cancellation_mode_override = outcome.callback_cancellation_mode_override;
            } else {
                return Ok(None);
            }
        }

        // §17 - On Hold
        "subscription.on_hold" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.as_deref()
                    .or(webhook.subscription_id.as_deref())
                    .unwrap_or("");
                let updated = repo.apply_subscription_transition(
                    app_id,
                    sub_id,
                    timestamp_epoch_ms,
                    SubscriptionWebhookTransition::OnHold,
                ).await?;
                if let Some(sub) = updated {
                    canonical_subscription = Some(sub.into());
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
                if let Ok(Some(sub)) = repo.get_subscription_by_sub_id(app_id, &sub_id).await {
                    if sub.status == "active" || sub.status == "trial" {
                        let updated = repo.apply_subscription_transition(
                            app_id,
                            &sub_id,
                            timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Paused,
                        ).await?;
                        if let Some(updated_sub) = updated {
                            canonical_subscription = Some(updated_sub.into());
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
            if let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_resumed(
                repo,
                app_id,
                &webhook,
                &fields,
                timestamp_epoch_ms,
            ).await? {
                canonical_subscription = outcome.canonical_subscription;
                if let Some(event_type) = outcome.callback_event_type {
                    callback_event_type = event_type;
                }
                callback_status_override = outcome.callback_status_override;
                callback_revocation_reason_override = outcome.callback_revocation_reason_override;
                callback_cancellation_mode_override = outcome.callback_cancellation_mode_override;
            }
        }

        // §20 - Cancellation Scheduled
        "subscription.cancellation_scheduled" => {
            if let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_cancellation_scheduled(
                repo,
                app_id,
                &webhook,
                &fields,
                timestamp_epoch_ms,
            ).await? {
                canonical_subscription = outcome.canonical_subscription;
                if let Some(event_type) = outcome.callback_event_type {
                    callback_event_type = event_type;
                }
                callback_status_override = outcome.callback_status_override;
                callback_revocation_reason_override = outcome.callback_revocation_reason_override;
                callback_cancellation_mode_override = outcome.callback_cancellation_mode_override;
            }
        }

        // §21 - Expired/Inactive
        "subscription.expired" => {
            if let Some(ref _user_id) = external_user_id {
                // Try to find subscription by purchase token first (more specific)
                if let Some(ref purchase_token) = fields.purchase_token {
                    if let Some(sub) = repo.get_subscription_by_purchase_token(app_id, purchase_token).await? {
                        let updated = repo.apply_subscription_transition(
                            app_id,
                            &sub.subscription_id,
                            timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Expired,
                        ).await?;
                        if let Some(updated_sub) = updated {
                            canonical_subscription = Some(updated_sub.into());
                            callback_event_type = "subscription.expired".to_string();
                            callback_status_override = Some("expired".to_string());
                        }
                    } else {
                        // Fallback to subscription_id if purchase token not found
                        let sub_id = fields.subscription_id.clone().unwrap_or_default();
                        let updated = repo.apply_subscription_transition(
                            app_id,
                            &sub_id,
                            timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Expired,
                        ).await?;
                        if let Some(updated_sub) = updated {
                            canonical_subscription = Some(updated_sub.into());
                            callback_event_type = "subscription.expired".to_string();
                            callback_status_override = Some("expired".to_string());
                        } else {
                            info!("Skipped stale expired event for subscription {}", sub_id);
                            return Ok(None);
                        }
                    }
                } else {
                    // Fallback to subscription_id if no purchase token
                    let sub_id = fields.subscription_id.clone().unwrap_or_default();
                    let updated = repo.apply_subscription_transition(
                        app_id,
                        &sub_id,
                        timestamp_epoch_ms,
                        SubscriptionWebhookTransition::Expired,
                    ).await?;
                    if let Some(updated_sub) = updated {
                        canonical_subscription = Some(updated_sub.into());
                        callback_event_type = "subscription.expired".to_string();
                        callback_status_override = Some("expired".to_string());
                    } else {
                        info!("Skipped stale expired event for subscription {}", sub_id);
                        return Ok(None);
                    }
                }
            }
        }

        // §22 - Cancelled
        "subscription.cancelled" => {
            if let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_cancelled_with_context(
                repo,
                app_id,
                &webhook,
                &fields,
                timestamp_epoch_ms,
            ).await? {
                canonical_subscription = outcome.canonical_subscription;
                if let Some(event_type) = outcome.callback_event_type {
                    callback_event_type = event_type;
                }
                callback_status_override = outcome.callback_status_override;
                callback_revocation_reason_override = outcome.callback_revocation_reason_override;
                callback_cancellation_mode_override = outcome.callback_cancellation_mode_override;
            }
        }

        // §23 - Order Created (payment pending)
        "payment.pending" => {
            if let Some(ref user_id) = external_user_id {
                let txn_id = fields.provider_transaction_id.as_deref()
                    .or(fields.subscription_id.as_deref())
                    .unwrap_or(&webhook_provider_webhook_id);
                repo.record_webhook_payment(WebhookPaymentRecordRequest {
                    app_id,
                    external_user_id: user_id,
                    provider: &provider,
                    provider_transaction_id: txn_id,
                    subscription_id: fields_subscription_id.as_deref(),
                    amount_cents: fields.amount_cents.unwrap_or(0),
                    status: "pending",
                })
                .await?;
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

                if repo.get_subscription_by_sub_id(app_id, sub_id).await?.is_none() {
                    warn!(
                        "Skipping order.failed event {}: subscription {} not found",
                        webhook.provider_webhook_id,
                        sub_id
                    );
                    return Ok(None);
                }

                let txn_id = fields.provider_transaction_id.as_deref()
                    .or(fields.subscription_id.as_deref())
                    .unwrap_or(&webhook_provider_webhook_id);
                repo.record_webhook_payment(WebhookPaymentRecordRequest {
                    app_id,
                    external_user_id: user_id,
                    provider: &provider,
                    provider_transaction_id: txn_id,
                    subscription_id: Some(sub_id),
                    amount_cents: fields.amount_cents.unwrap_or(0),
                    status: "failed",
                })
                .await?;

                let updated = repo
                    .apply_subscription_transition(
                        app_id,
                        sub_id,
                        timestamp_epoch_ms,
                        SubscriptionWebhookTransition::PaymentFailed,
                    )
                    .await?;

                let Some(updated_sub) = updated else {
                    warn!(
                        "Skipping stale order.failed update for subscription {}",
                        sub_id
                    );
                    return Ok(None);
                };

                canonical_subscription = Some(updated_sub.into());
            }
            callback_event_type = "payment.failed".to_string();
            callback_status_override = Some("failed".to_string());
        }

        // §25 - One-Time Product Purchased
        "purchase.one_time" => {
            if let Some(outcome) = crate::services::google_play::product_lifecycle::handle_otp_purchased(
                repo,
                app_id,
                &webhook,
                &fields,
                external_user_id.as_deref(),
                timestamp_epoch_ms,
            ).await? {
                canonical_subscription = outcome.canonical_subscription;
                if let Some(event_type) = outcome.callback_event_type {
                    callback_event_type = event_type;
                }
                callback_status_override = outcome.callback_status_override;
                callback_revocation_reason_override = outcome.callback_revocation_reason_override;
                callback_cancellation_mode_override = outcome.callback_cancellation_mode_override;
            }
        }

        // §26 - One-Time Product Cancelled
        "purchase.one_time_cancelled" => {
            if let Some(outcome) = crate::services::google_play::product_lifecycle::handle_otp_cancelled(
                repo,
                app_id,
                &webhook,
                &fields,
                external_user_id.as_deref(),
                timestamp_epoch_ms,
            ).await? {
                canonical_subscription = outcome.canonical_subscription;
                if let Some(event_type) = outcome.callback_event_type {
                    callback_event_type = event_type;
                }
                callback_status_override = outcome.callback_status_override;
                callback_revocation_reason_override = outcome.callback_revocation_reason_override;
                callback_cancellation_mode_override = outcome.callback_cancellation_mode_override;
            } else {
                return Ok(None);
            }
        }

        // §27 - Purchase Voided (Refund)
        "payment.refunded" => {
            if let Some(ref _user_id) = external_user_id {
                if let Some(token) = fields.purchase_token.as_deref().or(webhook.purchase_token.as_deref()) {
                    let existing = repo.get_payment_status(app_id, token).await?;
                    if existing.as_deref() != Some("refunded") {
                        repo.update_payment_status(app_id, token, "refunded").await?;
                    }
                    if let Some(sub) = repo.get_subscription_by_purchase_token(app_id, token).await? {
                        let updated = repo.apply_subscription_transition(
                            app_id,
                            &sub.subscription_id,
                            timestamp_epoch_ms,
                            SubscriptionWebhookTransition::Revoked {
                                revocation_reason: Some("REFUND".to_string()),
                            },
                        ).await?;
                        if let Some(updated_sub) = updated {
                            canonical_subscription = Some(updated_sub.into());
                        }
                    }
                } else if let Some(ref sub_id) = fields.subscription_id {
                    let updated = repo.apply_subscription_transition(
                        app_id,
                        sub_id,
                        timestamp_epoch_ms,
                        SubscriptionWebhookTransition::Revoked {
                            revocation_reason: Some("REFUND".to_string()),
                        },
                    ).await?;
                    if let Some(updated_sub) = updated {
                        canonical_subscription = Some(updated_sub.into());
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
                let inserted = repo.apply_topup_if_new(
                    app_id,
                    &user_id,
                    amount_cents,
                    &charge_id,
                ).await?;

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
            if let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_subscription_pending_purchase_cancelled(
                repo,
                app_id,
                &webhook,
                &fields,
                timestamp_epoch_ms,
            ).await? {
                canonical_subscription = outcome.canonical_subscription;
                if let Some(event_type) = outcome.callback_event_type {
                    callback_event_type = event_type;
                }
                callback_status_override = outcome.callback_status_override;
                callback_revocation_reason_override = outcome.callback_revocation_reason_override;
                callback_cancellation_mode_override = outcome.callback_cancellation_mode_override;
            } else {
                return Ok(None);
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
                    .unwrap_or(&webhook_provider_webhook_id);
                repo.record_webhook_payment(WebhookPaymentRecordRequest {
                    app_id,
                    external_user_id: user_id,
                    provider: &provider,
                    provider_transaction_id: txn_id,
                    subscription_id: fields_subscription_id.as_deref(),
                    amount_cents: fields.amount_cents.unwrap_or(0),
                    status: "dispute_created",
                })
                .await?;
            }
            callback_event_type = "dispute.created".to_string();
        }


        "subscription.updated" => {
            if let Some(ref _user_id) = external_user_id {
                let status = normalize_status(fields.status.as_deref());
                let sub_id = fields.subscription_id.clone()
                    .or(webhook.subscription_id.clone())
                    .unwrap_or_default();

                let subscription = repo
                    .commit_webhook_subscription(WebhookSubscriptionCommitRequest {
                        app_id,
                        external_user_id: _user_id,
                        subscription_id: &sub_id,
                        provider: &provider,
                        status: &status,
                        current_period_end: fields_current_period_end
                            .as_deref()
                            .and_then(parse_rfc3339_utc),
                        purchase_token: fields_purchase_token.as_deref(),
                        auto_renewing: fields.auto_renewing,
                        payment_state: None,
                        provider_customer_id: fields_provider_customer_id.as_deref(),
                        event_time_ms: timestamp_epoch_ms,
                        payment: None,
                        adopt_stale_payment: false,
                    })
                    .await?;

                let Some(subscription) = subscription else {
                    return Ok(None);
                };

                canonical_subscription = Some(subscription);
                    if let Some(new_event) = status_to_canonical_event(&status) {
                        callback_event_type = new_event;
                    }
                callback_status_override = Some(status.clone());
            }
        }

        "subscription.price_step_up" => {
            if let Some(outcome) = crate::services::google_play::subscription_lifecycle::handle_price_step_up_consent_required(
                repo,
                app_id,
                &webhook,
                &fields,
                timestamp_epoch_ms,
            ).await? {
                canonical_subscription = outcome.canonical_subscription;
                if let Some(event_type) = outcome.callback_event_type {
                    callback_event_type = event_type;
                }
                callback_status_override = outcome.callback_status_override;
                callback_revocation_reason_override = outcome.callback_revocation_reason_override;
                callback_cancellation_mode_override = outcome.callback_cancellation_mode_override;
            }
        }

        "subscription.pause_scheduled" => {
            if let Some(sub_id) = fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()) {
                let pause_scheduled_at = webhook.payload.pointer("/subscriptionNotification/pauseScheduleTimeMillis")
                    .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
                    .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis);

                if let Some(schedule_at) = pause_scheduled_at {
                    let updated = repo.apply_subscription_transition(
                        app_id,
                        sub_id,
                        timestamp_epoch_ms,
                        SubscriptionWebhookTransition::PauseScheduled {
                            google_pause_scheduled_at: schedule_at,
                        },
                    ).await?;

                    if let Some(updated_sub) = updated {
                        canonical_subscription = Some(updated_sub.into());
                        callback_status_override = Some("active".to_string());
                    } else {
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
                    let updated = repo.apply_subscription_transition(
                        app_id,
                        sub_id,
                        timestamp_epoch_ms,
                        SubscriptionWebhookTransition::Deferred {
                            google_deferred_until: until,
                        },
                    ).await?;

                    if updated.is_none() {
                        info!("Skipped stale deferred event for subscription {}", sub_id);
                    }
                }
            }
        }

        "subscription.price_changed" => {
            if let Some(ref user_id) = external_user_id {
                let txn_id = fields.provider_transaction_id.as_deref()
                    .or(fields.subscription_id.as_deref())
                    .unwrap_or(&webhook_provider_webhook_id);
                let _ = repo
                    .record_webhook_payment(WebhookPaymentRecordRequest {
                        app_id,
                        external_user_id: user_id,
                        provider: &provider,
                        provider_transaction_id: txn_id,
                        subscription_id: fields_subscription_id.as_deref(),
                        amount_cents: fields.amount_cents.unwrap_or(0),
                        status: "price_changed",
                    })
                    .await;
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
                let inserted = repo.apply_topup_if_new(
                    app_id,
                    &user_id,
                    amount_cents,
                    &charge_id,
                ).await?;

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

/// Normalize provider-specific event type to canonical format
/// Maps provider events per architecture doc section 3.4
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

fn callback_status_for_event(event_type: &str) -> Option<String> {
    match event_type {
        "subscription.activated" | "subscription.resumed" | "subscription.pause_scheduled" => {
            Some("active".to_string())
        }
        "subscription.grace_period" => Some("past_due".to_string()),
        "subscription.revoked" => Some("revoked".to_string()),
        "subscription.on_hold" => Some("on_hold".to_string()),
        "subscription.paused" => Some("paused".to_string()),
        "subscription.expired" => Some("expired".to_string()),
        "subscription.cancelled" => Some("cancelled".to_string()),
        _ => None,
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
    fn test_normalize_creem_checkout_completed_recurring_to_subscription_created() {
        let payload = serde_json::json!({
            "eventType": "checkout.completed",
            "object": {
                "billing_type": "recurring",
                "subscription": {
                    "id": "sub_001"
                }
            }
        });

        assert_eq!(
            normalize_event_type_with_payload("creem", "checkout.completed", Some(&payload)),
            "subscription.created"
        );
    }

    #[test]
    fn test_normalize_creem_checkout_completed_one_time_to_purchase_one_time() {
        let payload = serde_json::json!({
            "eventType": "checkout.completed",
            "object": {
                "billing_type": "one_time",
                "checkout_id": "co_001"
            }
        });

        assert_eq!(
            normalize_event_type_with_payload("creem", "checkout.completed", Some(&payload)),
            "purchase.one_time"
        );
    }

    #[test]
    fn test_creem_field_extraction_subscription_active() {
        let payload = serde_json::json!({
            "id": "evt_123",
            "eventType": "subscription.active",
            "createdAt": "2026-04-20T10:00:00Z",
            "object": {
                "id": "sub_456",
                "subscription_id": "sub_789",
                "product_id": "prod_premium",
                "status": "paid",
                "amount": 9999,
                "metadata": {
                    "user_id": "user_ext_001"
                }
            }
        });

        let webhook = WebhookProviderSnapshot {
            provider: "creem".to_string(),
            provider_webhook_id: "wh_123".to_string(),
            event_type: "subscription.active".to_string(),
            subscription_id: Some("sub_789".to_string()),
            purchase_token: None,
            payload,
            timestamp_epoch_ms: Some(1713607200000),
            suppressed: false,
            suppressed_reason: None,
        };

        let fields = extract_webhook_fields(&webhook);
        assert_eq!(fields.subscription_id, Some("sub_789".to_string()));
        assert_eq!(fields.product_id, Some("prod_premium".to_string()));
        assert_eq!(fields.status, Some("paid".to_string()));
        assert_eq!(fields.amount_cents, Some(9999));
    }

    #[test]
    fn test_creem_field_extraction_checkout_completed_recurring() {
        let payload = serde_json::json!({
            "id": "evt_co_123",
            "eventType": "checkout.completed",
            "createdAt": "2026-04-20T10:00:00Z",
            "object": {
                "id": "checkout_abc",
                "product_id": "prod_monthly",
                "billing_type": "recurring",
                "amount": 4999,
                "last_transaction_id": "txn_001",
                "subscription": {
                    "id": "sub_new_456",
                    "status": "paid",
                    "current_period_end_date": "2026-05-20T10:00:00Z",
                    "metadata": {
                        "user_id": "user_ext_002"
                    }
                }
            }
        });

        let webhook = WebhookProviderSnapshot {
            provider: "creem".to_string(),
            provider_webhook_id: "wh_456".to_string(),
            event_type: "checkout.completed".to_string(),
            subscription_id: Some("sub_new_456".to_string()),
            purchase_token: None,
            payload,
            timestamp_epoch_ms: Some(1713607200000),
            suppressed: false,
            suppressed_reason: None,
        };

        let fields = extract_webhook_fields(&webhook);
        assert_eq!(fields.subscription_id, Some("sub_new_456".to_string()));
        assert_eq!(fields.product_id, Some("prod_monthly".to_string()));
        assert_eq!(fields.status, Some("paid".to_string()));
        assert_eq!(fields.current_period_end, Some("2026-05-20T10:00:00Z".to_string()));
    }

    #[test]
    fn test_creem_field_extraction_checkout_completed_one_time() {
        let payload = serde_json::json!({
            "id": "evt_co_otp_123",
            "eventType": "checkout.completed",
            "createdAt": "2026-04-20T10:00:00Z",
            "object": {
                "id": "checkout_otp_001",
                "billing_type": "one_time",
                "product_id": "prod_lifetime",
                "checkout_id": "co_otp_001",
                "amount": 9999
            }
        });

        let webhook = WebhookProviderSnapshot {
            provider: "creem".to_string(),
            provider_webhook_id: "wh_otp_001".to_string(),
            event_type: "checkout.completed".to_string(),
            subscription_id: None,
            purchase_token: None,
            payload,
            timestamp_epoch_ms: Some(1713607200000),
            suppressed: false,
            suppressed_reason: None,
        };

        let fields = extract_webhook_fields(&webhook);
        assert_eq!(fields.subscription_id, Some("prod_lifetime".to_string()));
        assert_eq!(fields.purchase_token, Some("co_otp_001".to_string()));
        assert_eq!(fields.amount_cents, Some(9999));
    }

    #[test]
    fn test_creem_field_extraction_refund_with_amount_fallback() {
        let payload = serde_json::json!({
            "id": "evt_ref_123",
            "eventType": "refund.created",
            "createdAt": "2026-04-20T10:00:00Z",
            "object": {
                "id": "refund_789",
                "order_id": "order_original",
                "subscription_id": "sub_refunded",
                "last_transaction": {
                    "amount": 2999
                }
            }
        });

        let webhook = WebhookProviderSnapshot {
            provider: "creem".to_string(),
            provider_webhook_id: "wh_789".to_string(),
            event_type: "refund.created".to_string(),
            subscription_id: Some("sub_refunded".to_string()),
            purchase_token: None,
            payload,
            timestamp_epoch_ms: Some(1713607200000),
            suppressed: false,
            suppressed_reason: None,
        };

        let fields = extract_webhook_fields(&webhook);
        assert_eq!(fields.subscription_id, Some("sub_refunded".to_string()));
        assert_eq!(fields.purchase_token, Some("order_original".to_string()));
        assert_eq!(fields.amount_cents, Some(2999));
    }

    #[test]
    fn test_creem_metadata_user_id_from_checkout_path() {
        let payload = serde_json::json!({
            "id": "evt_123",
            "eventType": "checkout.completed",
            "createdAt": "2026-04-20T10:00:00Z",
            "object": {
                "id": "checkout_123",
                "checkout": {
                    "metadata": {
                        "user_id": "user_from_checkout"
                    }
                }
            }
        });

        assert_eq!(
            extract_metadata_user_id(&payload).as_deref(),
            Some("user_from_checkout")
        );
    }

    #[test]
    fn test_normalize_coinbase_special_events() {
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
    fn test_callback_status_for_pause_lifecycle_events() {
        assert_eq!(
            callback_status_for_event("subscription.pause_scheduled"),
            Some("active".to_string())
        );
        assert_eq!(
            callback_status_for_event("subscription.resumed"),
            Some("active".to_string())
        );
        assert_eq!(
            callback_status_for_event("subscription.paused"),
            Some("paused".to_string())
        );
        assert_eq!(callback_status_for_event("subscription.updated"), None);
    }

    #[test]
    fn test_mock_google_play_renewal_period_end_extends_existing_period() {
        let existing = chrono::DateTime::parse_from_rfc3339("2026-05-10T18:44:10Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        assert_eq!(
            mock_google_play_renewal_period_end(Some(existing)),
            chrono::DateTime::parse_from_rfc3339("2026-06-09T18:44:10Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        );
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
