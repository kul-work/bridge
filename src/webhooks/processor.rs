use crate::error::BridgeError;
use sqlx::PgPool;
use uuid::Uuid;
use tracing::info;

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
}

/// Process webhook: dedup, ordering, normalization
/// Used for webhook ingress processing across multiple providers.
#[allow(dead_code)]
pub async fn process_webhook(
    pool: &PgPool,
    webhook_provider_id: Uuid,
    app_id: Uuid,
) -> Result<Option<CanonicalWebhookPayload>, BridgeError> {
    // Get the webhook
    let webhook = crate::db::webhooks::get_webhook_provider(pool, webhook_provider_id)
        .await?;

    // Check if already suppressed
    if webhook.suppressed {
        info!(
            "Webhook {} already suppressed: {}",
            webhook_provider_id, webhook.suppressed_reason.as_deref().unwrap_or("unknown")
        );
        return Ok(None);
    }

    // Load app for slug
    let app = crate::db::apps::get_app(pool, app_id)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    // Load subscription to check event ordering and extract fields
    // Note: Skip stale event check if external_user_id is not available
    if let Some(_subscription_id) = &webhook.subscription_id {
        // TODO: Extract external_user_id from webhook payload
        // For now, stale event suppression requires external_user_id
    }

    // Normalize event to canonical type
    let event_type = normalize_event_type(&webhook.provider, &webhook.event_type);
    
    let timestamp_epoch_ms = webhook.timestamp_epoch_ms.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let timestamp_iso = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(|| chrono::Utc::now())
        .to_rfc3339();

    // Create canonical payload
    let canonical = CanonicalWebhookPayload {
        event_id: format!("{}-{}", webhook.provider, webhook.provider_webhook_id),
        event_type,
        timestamp: timestamp_iso,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: None,           // TODO: extract from payload
        subscription_id: webhook.subscription_id.clone(),
        external_user_id: None,     // TODO: load from subscription if available
        amount_cents: None,         // TODO: extract from payload
        auto_renewing: None,        // TODO: extract from payload
        purchase_token: None,       // TODO: extract from payload
        current_period_end: None,   // TODO: extract from payload
        status: None,               // TODO: extract from payload
        provider: webhook.provider.clone(),
        provider_event_id: webhook.provider_webhook_id.clone(),
    };

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
            _ => event_type.to_string(),
        },
        "lemonsqueezy" => match event_type {
            "subscription_created" => "subscription.created".to_string(),
            "subscription_updated" => "subscription.updated".to_string(),
            "subscription_expired" => "subscription.expired".to_string(),
            "subscription_cancelled" => "subscription.cancelled".to_string(),
            _ => event_type.to_string(),
        },
        "coinbase" => match event_type {
            "charge:confirmed" => "payment.succeeded".to_string(),
            "charge:failed" => "payment.failed".to_string(),
            _ => event_type.to_string(),
        },
        _ => event_type.to_string(),
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
    }
}
