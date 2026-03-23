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
/// Used for future webhook forwarding to app callbacks.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize)]
pub struct CanonicalWebhookPayload {
    pub event_id: String,
    pub event_type: String,
    pub timestamp: i64,
    pub app_id: String,
    pub subscription_id: Option<String>,
    pub external_user_id: Option<String>,
    pub status: Option<String>,
    pub provider: String,
    pub provider_event_id: String,
}

/// Process webhook: dedup, ordering, normalization
/// Used for future webhook ingress processing across multiple providers.
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

    // Load subscription to check event ordering
    if let Some(subscription_id) = &webhook.subscription_id {
        if let Ok(subscription) = crate::db::subscriptions::get_subscription(pool, app_id, subscription_id).await {
            // Check if this event is older than last processed event (stale event suppression)
            if let Some(timestamp_ms) = webhook.timestamp_epoch_ms {
                if timestamp_ms < subscription.last_event_time {
                    info!(
                        "Suppressing stale webhook {}: event_ts={} < last_event_ts={}",
                        webhook_provider_id, timestamp_ms, subscription.last_event_time
                    );
                    // Suppress and return None
                    crate::db::webhooks::suppress_webhook(pool, webhook_provider_id, "stale_ingress").await?;
                    return Ok(None);
                }
            }
        }
    }

    // Normalize event to canonical type
    let event_type = normalize_event_type(&webhook.provider, &webhook.event_type);
    
    // Create canonical payload
    let canonical = CanonicalWebhookPayload {
        event_id: format!("{}-{}", webhook.provider, webhook.provider_webhook_id),
        event_type: event_type.clone(),
        timestamp: webhook.timestamp_epoch_ms.unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
        app_id: app_id.to_string(),
        subscription_id: webhook.subscription_id.clone(),
        external_user_id: None, // TODO: load from subscription if available
        status: None, // TODO: extract from payload
        provider: webhook.provider.clone(),
        provider_event_id: webhook.provider_webhook_id.clone(),
    };

    Ok(Some(canonical))
}

/// Normalize provider-specific event type to canonical format
#[allow(dead_code)]
fn normalize_event_type(provider: &str, event_type: &str) -> String {
    match provider {
        "google_play" => match event_type {
            "SUBSCRIPTION_PURCHASED" | "SUBSCRIPTION_RENEWED" => "subscription.renewed".to_string(),
            "SUBSCRIPTION_CANCELED" => "subscription.cancelled".to_string(),
            "SUBSCRIPTION_RESTORED" => "subscription.renewed".to_string(),
            "SUBSCRIPTION_EXPIRED" => "subscription.expired".to_string(),
            "SUBSCRIPTION_ON_HOLD" => "subscription.paused".to_string(),
            _ => format!("google_play.{}", event_type),
        },
        "creem" => match event_type {
            "subscription.created" | "subscription.renewed" => "subscription.renewed".to_string(),
            "subscription.cancelled" => "subscription.cancelled".to_string(),
            "subscription.expired" => "subscription.expired".to_string(),
            _ => event_type.to_string(),
        },
        "lemonsqueezy" => match event_type {
            "subscription_created" | "subscription_updated" => "subscription.renewed".to_string(),
            "subscription_cancelled" => "subscription.cancelled".to_string(),
            "subscription_expired" => "subscription.expired".to_string(),
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
            "subscription.renewed"
        );
        assert_eq!(
            normalize_event_type("google_play", "SUBSCRIPTION_CANCELED"),
            "subscription.cancelled"
        );
    }

    #[test]
    fn test_normalize_creem_events() {
        assert_eq!(
            normalize_event_type("creem", "subscription.created"),
            "subscription.renewed"
        );
        assert_eq!(
            normalize_event_type("creem", "subscription.cancelled"),
            "subscription.cancelled"
        );
    }
}
