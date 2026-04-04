use crate::error::BridgeError;
use std::time::Duration;
use sqlx::PgPool;
use uuid::Uuid;
use reqwest::Client;
use tracing::{info, warn, error};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use chrono::Utc;

#[allow(dead_code)]
type HmacSha256 = Hmac<Sha256>;

const WEBHOOK_FORWARD_TIMEOUT_SECS: u64 = 10;

/// Forward webhook to app callback URL with HMAC signature
/// Used for future webhook delivery to app callbacks.
#[allow(dead_code)]
pub async fn forward_webhook(
    pool: &PgPool,
    app_id: Uuid,
    webhook_delivery_id: Uuid,
    payload: crate::webhooks::processor::CanonicalWebhookPayload,
) -> Result<(), BridgeError> {
    // Get app details
    let app = crate::db::apps::get_app(pool, app_id).await?;

    // Get delivery record
    let delivery = crate::db::webhooks::get_webhook_delivery(pool, webhook_delivery_id).await?;

    // Don't retry if already forwarded or max attempts reached
    if delivery.forwarded || delivery.forward_attempts >= 3 {
        return Ok(());
    }

    if let Some(ref subscription_id) = payload.subscription_id {
        if let Some(subscription) = crate::db::subscriptions::get_subscription_by_sub_id(
            pool,
            app_id,
            subscription_id,
        )
        .await?
        {
            if payload.timestamp_epoch_ms < subscription.last_event_time {
                crate::db::webhooks::suppress_webhook(
                    pool,
                    delivery.webhook_provider_id,
                    "superseded_before_forward",
                )
                .await?;
                crate::db::webhooks::update_webhook_delivery_attempt(
                    pool,
                    webhook_delivery_id,
                    None,
                    Some("Suppressed stale event before forward".to_string()),
                    true,
                )
                .await?;
                info!(
                    "Suppressed stale webhook delivery {} for subscription {} (event ts={} < last_event_time={})",
                    webhook_delivery_id,
                    subscription_id,
                    payload.timestamp_epoch_ms,
                    subscription.last_event_time
                );
                return Ok(());
            }
        }
    }

    if delivery.forward_attempts > 0 {
        info!(
            "Retrying webhook delivery {} (attempt {} of 3)",
            webhook_delivery_id,
            delivery.forward_attempts + 1,
        );
    }

    // Serialize payload
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| BridgeError::WebhookError(format!("Failed to serialize payload: {}", e)))?;

    // Create HMAC signature
    let timestamp = Utc::now().timestamp().to_string();
    let signature = create_signature(&payload_json, &app.webhook_callback_secret)?;

    // Make HTTP request
    let client = Client::builder()
        .timeout(Duration::from_secs(WEBHOOK_FORWARD_TIMEOUT_SECS))
        .build()
        .map_err(|e| BridgeError::WebhookError(format!("Failed to build webhook client: {}", e)))?;
    let response = client
        .post(&app.webhook_callback_url)
        .header("X-Pay-Signature", &signature)
        .header("X-Pay-Timestamp", &timestamp)
        .header("X-Pay-Event-Id", &payload.event_id)
        .header("Content-Type", "application/json")
        .body(payload_json)
        .send()
        .await;

    // Handle response
    match response {
        Ok(resp) => {
            let status = resp.status().as_u16() as i32;
            let is_success = resp.status().is_success();

            if is_success {
                info!(
                    "Successfully forwarded webhook {} to app {} (status: {})",
                    webhook_delivery_id, app_id, status
                );
                crate::db::webhooks::update_webhook_delivery_attempt(
                    pool,
                    webhook_delivery_id,
                    Some(status),
                    None,
                    true,
                )
                .await?;
            } else {
                let error_msg = format!("HTTP {}", status);
                warn!(
                    "Failed to forward webhook {} to app {}: {}",
                    webhook_delivery_id, app_id, error_msg
                );
                crate::db::webhooks::update_webhook_delivery_attempt(
                    pool,
                    webhook_delivery_id,
                    Some(status),
                    Some(error_msg),
                    false,
                )
                .await?;
            }
        }
        Err(e) => {
            let error_msg = format!("Request error: {}", e);
            error!(
                "Failed to forward webhook {} to app {}: {}",
                webhook_delivery_id, app_id, error_msg
            );
            crate::db::webhooks::update_webhook_delivery_attempt(
                pool,
                webhook_delivery_id,
                None,
                Some(error_msg),
                false,
            )
            .await?;
        }
    }

    Ok(())
}

/// Create HMAC-SHA256 signature for webhook
#[allow(dead_code)]
fn create_signature(payload: &str, secret: &str) -> Result<String, BridgeError> {
    // Signature format: HMAC-SHA256(secret, raw JSON payload)
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;
    
    mac.update(payload.as_bytes());
    let result = mac.finalize();
    
    Ok(format!("sha256={}", hex::encode(result.into_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_signature() {
        let result = create_signature(
            r#"{"event_id":"test"}"#,
            "secret"
        ).unwrap();
        
        assert!(result.starts_with("sha256="));
    }

    #[test]
    fn test_signature_deterministic() {
        let sig1 = create_signature(r#"{"event_id":"test"}"#, "secret").unwrap();
        let sig2 = create_signature(r#"{"event_id":"test"}"#, "secret").unwrap();
        
        assert_eq!(sig1, sig2);
    }
}
