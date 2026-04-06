use crate::error::BridgeError;
use sqlx::PgPool;
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
            google_new_price_cents: None,
            google_price_step_up_consent_deadline: None,
        },
    }
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

    let fields = extract_webhook_fields(&webhook);
    let canonical_event = normalize_event_type(&webhook.provider, &webhook.event_type);
    let timestamp_epoch_ms = webhook
        .timestamp_epoch_ms
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let timestamp_iso = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    Ok(Some(CanonicalWebhookPayload {
        event_id: format!("{}-{}", webhook.provider, webhook.provider_webhook_id),
        event_type: canonical_event,
        timestamp: timestamp_iso,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: fields.product_id,
        subscription_id: fields.subscription_id.or(webhook.subscription_id.clone()),
        external_user_id,
        amount_cents: fields.amount_cents,
        auto_renewing: fields.auto_renewing,
        purchase_token: fields.purchase_token.or(webhook.purchase_token.clone()),
        current_period_end: fields.current_period_end,
        status: fields.status,
        provider: webhook.provider.clone(),
        provider_event_id: webhook.provider_webhook_id.clone(),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: None,
        cancellation_mode: None,
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

    let mut canonical_event = normalize_event_type(&webhook.provider, &webhook.event_type);

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

    let fields = extract_webhook_fields(&webhook);
    let timestamp_epoch_ms = webhook.timestamp_epoch_ms.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let timestamp_iso = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

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
                let applied = crate::db::subscriptions::update_subscription_status(
                    pool, app_id, sub_id, "pending", timestamp_epoch_ms,
                ).await?;
                if !applied {
                    info!("Skipped stale pending event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
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
                    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                }
            }
        }

        "subscription.grace_period" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                let applied = crate::db::subscriptions::update_subscription_status(
                    pool, app_id,
                    &sub_id,
                    "past_due", timestamp_epoch_ms,
                ).await?;
                if !applied {
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
                let applied = crate::db::subscriptions::update_subscription_status(
                    pool, app_id, sub_id, "revoked", timestamp_epoch_ms,
                ).await?;
                if !applied {
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
                let applied = crate::db::subscriptions::update_subscription_status(
                    pool, app_id, sub_id, "on_hold", timestamp_epoch_ms,
                ).await?;
                if !applied {
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
                        let applied = crate::db::subscriptions::update_subscription_status(
                            pool, app_id, &sub_id, "paused", timestamp_epoch_ms,
                        ).await?;
                        if !applied {
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
                        let applied = crate::db::subscriptions::update_subscription_status(
                            pool, app_id, &sub_id, "active", timestamp_epoch_ms,
                        ).await?;
                        if !applied {
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
                    let result = sqlx::query(
                        "UPDATE pay.subscriptions
                         SET auto_renewing = false,
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
                        info!("Skipped stale cancellation_scheduled event for subscription {}", sub_id);
                    }
                }
            }
        }

        // §21 - Expired/Inactive
        "subscription.expired" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                let applied = crate::db::subscriptions::update_subscription_status(
                    pool, app_id,
                    &sub_id,
                    "expired", timestamp_epoch_ms,
                ).await?;
                if !applied {
                    info!("Skipped stale expired event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        // §22 - Cancelled
        "subscription.cancelled" => {
            if let Some(ref _user_id) = external_user_id {
                let sub_id = fields.subscription_id.clone().unwrap_or_default();
                let applied = crate::db::subscriptions::update_subscription_status(
                    pool, app_id,
                    &sub_id,
                    "cancelled", timestamp_epoch_ms,
                ).await?;
                if !applied {
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
        }

        // §24 - Order Failed
        "payment.failed" if webhook.provider == "coinbase" => {
            let charge_id = fields.provider_transaction_id
                .as_deref()
                .or(webhook.subscription_id.as_deref())
                .unwrap_or(&webhook.provider_webhook_id);
            info!("Coinbase charge failed: charge_id={}", charge_id);
        }

        "payment.failed" => {
            if let Some(ref user_id) = external_user_id {
                let txn_id = fields.provider_transaction_id.as_deref()
                    .or(fields.subscription_id.as_deref())
                    .unwrap_or(&webhook.provider_webhook_id);
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                crate::db::payments::record_payment_tx(
                    &mut tx, app_id, user_id, &webhook.provider, txn_id,
                    fields.subscription_id.as_deref(),
                    fields.amount_cents.unwrap_or(0), "failed",
                ).await?;
                tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            }
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
        }

        // §26 - One-Time Product Cancelled
        "purchase.one_time_cancelled" => {
            if let Some(ref user_id) = external_user_id {
                let token = fields.purchase_token.as_deref()
                    .or(fields.provider_transaction_id.as_deref())
                    .unwrap_or("");
                let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
                crate::db::payments::record_payment_tx(
                    &mut tx, app_id, user_id, &webhook.provider, token,
                    fields.subscription_id.as_deref(),
                    fields.amount_cents.unwrap_or(0), "cancelled",
                ).await?;
                tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
            }
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
                        crate::db::subscriptions::update_subscription_status(
                            pool, app_id, &sub.subscription_id, "revoked", timestamp_epoch_ms,
                        ).await?;
                    }
                } else if let Some(ref sub_id) = fields.subscription_id {
                     crate::db::subscriptions::update_subscription_status(
                        pool, app_id, sub_id, "revoked", timestamp_epoch_ms,
                    ).await?;
                }
            }
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
                let applied = crate::db::subscriptions::update_subscription_status(
                    pool, app_id,
                    &sub_id,
                    "cancelled", timestamp_epoch_ms,
                ).await?;
                if !applied {
                    info!("Skipped stale pending_purchase_cancelled event for subscription {}", sub_id);
                    return Ok(None);
                }
            }
        }

        // §29 - Dispute Created (admin alert + app callback)
        "dispute.created" => {
            // Send admin alert (email to Tyde support)
            info!(
                "Admin alert: dispute created for app_id={} provider={} event_id={} amount={:?}",
                app_id, webhook.provider, webhook.provider_webhook_id, fields.amount_cents
            );

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
        }


        "subscription.updated" => {
            if let Some(ref _user_id) = external_user_id {
                let status = normalize_status(fields.status.as_deref());
                let sub_id = fields.subscription_id.clone()
                    .or(webhook.subscription_id.clone())
                    .unwrap_or_default();
                
                let applied = crate::db::subscriptions::update_subscription_status(
                    pool,
                    app_id,
                    &sub_id,
                    &status,
                    timestamp_epoch_ms,
                ).await?;

                if applied {
                    if let Some(new_event) = status_to_canonical_event(&status) {
                        canonical_event = new_event;
                    }
                } else {
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

    // Step 5: Build canonical payload with real data
    let canonical = CanonicalWebhookPayload {
        event_id: format!("{}-{}", webhook.provider, webhook.provider_webhook_id),
        event_type: canonical_event,
        timestamp: timestamp_iso,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: fields.product_id,
        subscription_id: fields.subscription_id.or(webhook.subscription_id.clone()),
        external_user_id,
        amount_cents: fields.amount_cents,
        auto_renewing: fields.auto_renewing,
        purchase_token: fields.purchase_token.or(webhook.purchase_token.clone()),
        current_period_end: fields.current_period_end,
        status: fields.status,
        provider: webhook.provider.clone(),
        provider_event_id: webhook.provider_webhook_id.clone(),
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: fields.cancel_reason,
        cancellation_mode: None,
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
