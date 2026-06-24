use crate::db::webhooks::WebhookDelivery;
use crate::error::BridgeError;
use crate::ports::{AppLookupRepository, WebhookForwardRepository, WebhookWriteRepository};
use crate::utils::diagnostic_hash;
use crate::webhooks::processor::CanonicalWebhookPayload;
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;
use std::time::Duration;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const WEBHOOK_FORWARD_TIMEOUT_SECS: u64 = 10;

/// Forward webhook to app callback URL with HMAC signature
/// Used for future webhook delivery to app callbacks.
pub async fn forward_webhook<R: WebhookForwardRepository + AppLookupRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook_delivery_id: Uuid,
    payload: CanonicalWebhookPayload,
) -> Result<(), BridgeError> {
    // Get app details
    let app = repo.get_app(app_id).await?;

    // Get delivery record
    let delivery = repo.get_webhook_delivery(webhook_delivery_id).await?;

    // Don't retry if already forwarded or already dead-lettered.
    if delivery.forwarded || delivery.dead_lettered || delivery.forward_attempts >= 3 {
        if delivery.dead_lettered {
            info!(
                "Skipping dead-lettered webhook delivery {}",
                webhook_delivery_id
            );
        }
        return Ok(());
    }

    if let Some(ref subscription_id) = payload.subscription_id {
        if let Some(subscription) = repo
            .get_subscription_by_sub_id(app_id, subscription_id)
            .await?
        {
            if payload.timestamp_epoch_ms < subscription.last_event_time {
                repo.suppress_webhook(delivery.webhook_provider_id, "superseded_before_forward")
                    .await?;
                repo.update_webhook_delivery_attempt(
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
            app_id = %app_id,
            webhook_delivery_id = %webhook_delivery_id,
            provider = %payload.provider,
            event_type = %payload.event_type,
            attempt = delivery.forward_attempts + 1,
            "Retrying webhook delivery"
        );
    } else {
        info!(
            app_id = %app_id,
            webhook_delivery_id = %webhook_delivery_id,
            provider = %payload.provider,
            event_type = %payload.event_type,
            attempt = 1,
            "Forwarding webhook delivery first attempt"
        );
    }

    // Serialize payload
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| BridgeError::WebhookError(format!("Failed to serialize payload: {}", e)))?;
    let diagnostic_payload_json = serialize_diagnostic_payload(&payload)?;

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
                    app_id = %app_id,
                    webhook_delivery_id = %webhook_delivery_id,
                    provider = %payload.provider,
                    event_type = %payload.event_type,
                    provider_event_id = %payload.provider_event_id,
                    external_user_id = payload.external_user_id.as_deref(),
                    status,
                    "Successfully forwarded webhook to app"
                );
                repo.update_webhook_delivery_attempt(webhook_delivery_id, Some(status), None, true)
                    .await?;
            } else {
                let _response_body = resp
                    .text()
                    .await
                    .unwrap_or_else(|e| format!("(failed to read response body: {})", e));
                let error_msg = format_http_failure(status);
                debug!(
                    webhook_delivery_id = %webhook_delivery_id,
                    app_id = %app_id,
                    callback_url = %app.webhook_callback_url,
                    http_status = status,
                    outbound_payload = %diagnostic_payload_json,
                    response_body = "[omitted]",
                    "Webhook forwarding diagnostics"
                );
                let updated_delivery = repo
                    .update_webhook_delivery_attempt(
                        webhook_delivery_id,
                        Some(status),
                        Some(error_msg),
                        false,
                    )
                    .await?;
                log_persisted_delivery_failure(
                    app_id,
                    webhook_delivery_id,
                    &payload,
                    &updated_delivery,
                    DeliveryFailureLogContext {
                        last_http_status: Some(status),
                        error_kind: "app_non_success_status",
                        retry_message: "Failed to forward webhook to app",
                        dead_letter_message: "Webhook delivery permanently failed and marked dead_lettered",
                    },
                );
            }
        }
        Err(e) => {
            let error_msg = format!("Request error: {}", e);
            debug!(
                webhook_delivery_id = %webhook_delivery_id,
                app_id = %app_id,
                callback_url = %app.webhook_callback_url,
                outbound_payload = %diagnostic_payload_json,
                error = %error_msg,
                "Webhook forwarding diagnostics"
            );
            let updated_delivery = repo
                .update_webhook_delivery_attempt(webhook_delivery_id, None, Some(error_msg), false)
                .await?;
            log_persisted_delivery_failure(
                app_id,
                webhook_delivery_id,
                &payload,
                &updated_delivery,
                DeliveryFailureLogContext {
                    last_http_status: None,
                    error_kind: "request_error",
                    retry_message: "Failed to forward webhook to app due to request error",
                    dead_letter_message: "Webhook delivery permanently failed and marked dead_lettered",
                },
            );
        }
    }

    Ok(())
}

fn log_persisted_delivery_failure(
    app_id: Uuid,
    webhook_delivery_id: Uuid,
    payload: &CanonicalWebhookPayload,
    delivery: &WebhookDelivery,
    context: DeliveryFailureLogContext,
) {
    if delivery.dead_lettered {
        error!(
            signal_class = "alert_signal",
            alert_key = "bridge.webhook.dead_lettered",
            alert_severity = "ticket",
            alert_subject = "Webhook delivery dead-lettered",
            app_id = %app_id,
            webhook_delivery_id = %webhook_delivery_id,
            provider = %payload.provider,
            event_type = %payload.event_type,
            provider_event_id = %payload.provider_event_id,
            attempts = delivery.forward_attempts,
            last_http_status = context.last_http_status,
            error_kind = context.error_kind,
            error_msg = ?delivery.last_error,
            dead_letter_reason = ?delivery.dead_letter_reason,
            dead_lettered_at = ?delivery.dead_lettered_at,
            failure_message = context.dead_letter_message,
            "Webhook delivery permanently failed and marked dead_lettered"
        );
    } else {
        warn!(
            signal_class = "support_debug_signal",
            alert_key = "bridge.callback.delivery_failed",
            alert_severity = "audit",
            alert_subject = "Webhook delivery retryable failure",
            app_id = %app_id,
            webhook_delivery_id = %webhook_delivery_id,
            provider = %payload.provider,
            event_type = %payload.event_type,
            provider_event_id = %payload.provider_event_id,
            attempt = delivery.forward_attempts,
            attempts = delivery.forward_attempts,
            last_http_status = context.last_http_status,
            error_kind = context.error_kind,
            error_msg = ?delivery.last_error,
            failure_message = context.retry_message,
            "Webhook delivery retryable failure persisted"
        );
    }
}

struct DeliveryFailureLogContext {
    last_http_status: Option<i32>,
    error_kind: &'static str,
    retry_message: &'static str,
    dead_letter_message: &'static str,
}

fn serialize_diagnostic_payload(payload: &CanonicalWebhookPayload) -> Result<String, BridgeError> {
    serde_json::to_string(&scrub_payload_for_diagnostics(payload)).map_err(|e| {
        BridgeError::WebhookError(format!("Failed to serialize diagnostic payload: {}", e))
    })
}

fn scrub_payload_for_diagnostics(payload: &CanonicalWebhookPayload) -> CanonicalWebhookPayload {
    let mut scrubbed = payload.clone();
    scrubbed.purchase_token = scrubbed.purchase_token.map(|token| diagnostic_hash(&token));
    scrubbed
}

fn format_http_failure(status: i32) -> String {
    format!("HTTP error status {}", status)
}

/// Create a webhook delivery and forward it in one step.
pub async fn queue_and_forward_webhook<
    R: AppLookupRepository + WebhookForwardRepository + WebhookWriteRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    webhook_provider_id: Uuid,
    payload: crate::webhooks::processor::CanonicalWebhookPayload,
) -> Result<(), BridgeError> {
    let delivery = repo
        .create_webhook_delivery(app_id, webhook_provider_id)
        .await?;

    if !delivery.created {
        info!(
            "Webhook delivery already queued for provider webhook {} (delivery={}); treating duplicate as idempotent",
            webhook_provider_id, delivery.id,
        );
        return Ok(());
    }

    forward_webhook(repo, app_id, delivery.id, payload).await
}

/// Create a webhook provider record, enqueue a delivery, and forward it.
#[allow(clippy::too_many_arguments)]
pub async fn create_and_forward_webhook<
    R: AppLookupRepository + WebhookForwardRepository + WebhookWriteRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    provider: &str,
    provider_webhook_id: &str,
    event_type: &str,
    subscription_id: Option<String>,
    purchase_token: Option<String>,
    provider_payload: serde_json::Value,
    timestamp_epoch_ms: Option<i64>,
    canonical_payload: crate::webhooks::processor::CanonicalWebhookPayload,
) -> Result<(), BridgeError> {
    let (webhook_provider_id, is_new) = repo
        .create_webhook_provider(
            app_id,
            provider,
            provider_webhook_id,
            event_type,
            subscription_id,
            purchase_token,
            provider_payload,
            timestamp_epoch_ms,
        )
        .await?;

    if !is_new {
        info!(
            "Skipping duplicate synthetic webhook delivery: app_id={}, provider={}, provider_event_id={}, event_type={}",
            app_id, provider, provider_webhook_id, event_type
        );
        return Ok(());
    }

    queue_and_forward_webhook(repo, app_id, webhook_provider_id, canonical_payload).await
}

/// Create HMAC-SHA256 signature for webhook
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
        let result = create_signature(r#"{"event_id":"test"}"#, "secret").unwrap();

        assert!(result.starts_with("sha256="));
    }

    #[test]
    fn test_signature_deterministic() {
        let sig1 = create_signature(r#"{"event_id":"test"}"#, "secret").unwrap();
        let sig2 = create_signature(r#"{"event_id":"test"}"#, "secret").unwrap();

        assert_eq!(sig1, sig2);
    }
}
