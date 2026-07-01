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
pub const WEBHOOK_DELIVERY_LEASE_SECS: i64 = 600;

pub fn webhook_worker_id(prefix: &str) -> String {
    format!("{}-{}", prefix, Uuid::new_v4())
}

async fn refresh_delivery_claim<R: WebhookForwardRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook_delivery_id: Uuid,
    claim_token: Uuid,
    payload: &CanonicalWebhookPayload,
    side_effect: &str,
) -> Result<bool, BridgeError> {
    let refreshed = repo
        .refresh_webhook_delivery_claim(
            app_id,
            webhook_delivery_id,
            claim_token,
            WEBHOOK_DELIVERY_LEASE_SECS,
        )
        .await?;

    if !refreshed {
        info!(
            app_id = %app_id,
            webhook_delivery_id = %webhook_delivery_id,
            provider = %payload.provider,
            event_type = %payload.event_type,
            provider_event_id = %payload.provider_event_id,
            side_effect,
            "Skipping webhook delivery side effect because claim was lost"
        );
    }

    Ok(refreshed)
}

/// Forward webhook to app callback URL with HMAC signature
/// Used for future webhook delivery to app callbacks.
pub async fn forward_webhook<R: WebhookForwardRepository + AppLookupRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook_delivery_id: Uuid,
    claim_token: Uuid,
    payload: CanonicalWebhookPayload,
) -> Result<(), BridgeError> {
    // Get app details
    let app = repo.get_app(app_id).await?;

    // Get delivery record
    let delivery = repo.get_webhook_delivery(webhook_delivery_id).await?;

    // Don't retry if already forwarded or already dead-lettered.
    if delivery.forwarded || delivery.dead_lettered || delivery.forward_attempts >= 3 {
        info!(
            app_id = %app_id,
            webhook_delivery_id = %webhook_delivery_id,
            provider = %payload.provider,
            event_type = %payload.event_type,
            provider_event_id = %payload.provider_event_id,
            external_user_id_hash = payload
                .external_user_id
                .as_deref()
                .map(diagnostic_hash)
                .as_deref(),
            forwarded = delivery.forwarded,
            dead_lettered = delivery.dead_lettered,
            attempts = delivery.forward_attempts,
            outcome = "terminal_skip",
            "Skipping terminal webhook delivery"
        );
        return Ok(());
    }

    // Resolve the stored row to compare staleness against.
    //
    // Google Play subscription_id is a shared product SKU, so a sub_id lookup
    // could measure user A's event against user B's newer row and silently
    // drop it. For google_play, key the stale check on the purchase token,
    // which uniquely identifies the user's subscription. If the token is
    // absent or unmatched, fail open and forward rather than suppressing
    // against a wrong same-SKU row. Other providers keep sub_id keying, where
    // that id is already user-unique and purchase_token may be null.
    let stale_check_subscription = if payload.provider == "google_play" {
        match payload.purchase_token.as_deref() {
            Some(token) => repo.get_subscription_by_purchase_token(app_id, token).await?,
            None => None,
        }
    } else if let Some(ref subscription_id) = payload.subscription_id {
        repo.get_subscription_by_sub_id(app_id, subscription_id).await?
    } else {
        None
    };

    if let Some(subscription) = stale_check_subscription {
        if payload.timestamp_epoch_ms < subscription.last_event_time {
            if !refresh_delivery_claim(
                repo,
                app_id,
                webhook_delivery_id,
                claim_token,
                &payload,
                "stale_suppression",
            )
            .await?
            {
                return Ok(());
            }
            repo.suppress_webhook(delivery.webhook_provider_id, "superseded_before_forward")
                .await?;
            repo.complete_webhook_delivery_attempt(
                webhook_delivery_id,
                claim_token,
                None,
                Some("Suppressed stale event before forward".to_string()),
                true,
            )
            .await?;
            info!(
                app_id = %app_id,
                webhook_delivery_id = %webhook_delivery_id,
                webhook_provider_id = %delivery.webhook_provider_id,
                provider = %payload.provider,
                event_type = %payload.event_type,
                provider_event_id = %payload.provider_event_id,
                external_user_id_hash = payload
                    .external_user_id
                    .as_deref()
                    .map(diagnostic_hash)
                    .as_deref(),
                subscription_id = payload.subscription_id.as_deref(),
                event_time_ms = payload.timestamp_epoch_ms,
                last_event_time = subscription.last_event_time,
                outcome = "suppressed_stale",
                "Suppressed stale webhook delivery before forward"
            );
            return Ok(());
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

    if !refresh_delivery_claim(
        repo,
        app_id,
        webhook_delivery_id,
        claim_token,
        &payload,
        "callback_post",
    )
    .await?
    {
        return Ok(());
    }

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
                    external_user_id_hash = payload
                        .external_user_id
                        .as_deref()
                        .map(diagnostic_hash)
                        .as_deref(),
                    status,
                    "Successfully forwarded webhook to app"
                );
                repo.complete_webhook_delivery_attempt(
                    webhook_delivery_id,
                    claim_token,
                    Some(status),
                    None,
                    true,
                )
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
                    .complete_webhook_delivery_attempt(
                        webhook_delivery_id,
                        claim_token,
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
                .complete_webhook_delivery_attempt(
                    webhook_delivery_id,
                    claim_token,
                    None,
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
    scrubbed.external_user_id = scrubbed.external_user_id.map(|user_id| diagnostic_hash(&user_id));
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
    let worker_id = webhook_worker_id("queue-forward");
    let delivery = repo
        .create_webhook_delivery(
            app_id,
            webhook_provider_id,
            &worker_id,
            WEBHOOK_DELIVERY_LEASE_SECS,
        )
        .await?;

    if !delivery.created {
        info!(
            app_id = %app_id,
            webhook_provider_id = %webhook_provider_id,
            webhook_delivery_id = %delivery.id,
            provider = %payload.provider,
            event_type = %payload.event_type,
            provider_event_id = %payload.provider_event_id,
            external_user_id_hash = payload
                .external_user_id
                .as_deref()
                .map(diagnostic_hash)
                .as_deref(),
            outcome = "duplicate_delivery_queued",
            "Webhook delivery already queued; treating duplicate as idempotent"
        );
        return Ok(());
    }

    let canonical_payload = serde_json::to_value(&payload)
        .map_err(|e| BridgeError::InternalServerError(e.to_string()))?;
    repo.store_webhook_delivery_canonical_payload_and_mark_processed(
        app_id,
        delivery.id,
        webhook_provider_id,
        canonical_payload,
    )
    .await?;

    let claim_token = delivery.claim_token.ok_or_else(|| {
        BridgeError::InternalServerError("new webhook delivery was not claimed".to_string())
    })?;

    forward_webhook(repo, app_id, delivery.id, claim_token, payload).await
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
    let canonical_payload_value = serde_json::to_value(&canonical_payload)
        .map_err(|e| BridgeError::InternalServerError(e.to_string()))?;

    let delivery = repo
        .create_synthetic_webhook_delivery(
            app_id,
            provider,
            provider_webhook_id,
            event_type,
            subscription_id,
            purchase_token,
            provider_payload,
            timestamp_epoch_ms,
            canonical_payload_value,
            &webhook_worker_id("synthetic"),
            WEBHOOK_DELIVERY_LEASE_SECS,
        )
        .await?;

    if !delivery.created {
        info!(
            app_id = %app_id,
            provider,
            provider_event_id = provider_webhook_id,
            event_type,
            outcome = "duplicate_synthetic_webhook",
            "Skipping duplicate synthetic webhook delivery"
        );
        return Ok(());
    }

    let claim_token = delivery.claim_token.ok_or_else(|| {
        BridgeError::InternalServerError("new synthetic webhook delivery was not claimed".to_string())
    })?;

    forward_webhook(repo, app_id, delivery.id, claim_token, canonical_payload).await
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

    #[test]
    fn diagnostic_payload_hashes_user_and_purchase_token() {
        let payload = CanonicalWebhookPayload {
            event_id: "evt_1".to_string(),
            event_type: "subscription.activated".to_string(),
            timestamp: "2026-06-24T00:00:00Z".to_string(),
            timestamp_epoch_ms: 1782259200000,
            app_slug: "household".to_string(),
            product_id: Some("monthly".to_string()),
            subscription_id: Some("sub_1".to_string()),
            external_user_id: Some("user_raw".to_string()),
            amount_cents: Some(499),
            new_price_cents: None,
            auto_renewing: Some(true),
            purchase_token: Some("purchase_token_raw".to_string()),
            current_period_end: None,
            status: Some("active".to_string()),
            provider: "google_play".to_string(),
            provider_event_id: "provider_evt_1".to_string(),
            previous_status: None,
            corrected_status: None,
            reconciliation_source: None,
            revocation_reason: None,
            cancellation_mode: None,
            google_price_step_up_consent_deadline: None,
            google_pause_scheduled_at: None,
            google_deferred_until: None,
            google_pending_price_change_new_price_cents: None,
            google_pending_price_change_currency: None,
            google_pending_price_change_mode: None,
            google_pending_price_change_state: None,
            google_pending_price_change_expected_at: None,
        };

        let scrubbed = scrub_payload_for_diagnostics(&payload);

        let expected_user_hash = diagnostic_hash("user_raw");
        let expected_token_hash = diagnostic_hash("purchase_token_raw");

        assert_eq!(scrubbed.external_user_id.as_deref(), Some(expected_user_hash.as_str()));
        assert_eq!(scrubbed.purchase_token.as_deref(), Some(expected_token_hash.as_str()));
    }
}

/// Branch coverage for the Google identity cross-user fix in `forward_webhook`'s
/// stale-suppression lookup (Path 2). Uses an in-memory mock repository so the
/// provider-aware decision actually executes:
/// - google_play compares staleness against the row matching the purchase token
/// - a token miss fails open (forwards instead of suppressing)
/// - other providers keep subscription_id keying
///
/// "Forward" cases reach an HTTP attempt to a closed local port (connection
/// refused), which `forward_webhook` tolerates; we assert on whether the
/// webhook was suppressed, which is the decision under test.
#[cfg(test)]
mod stale_suppression_branch_tests {
    use super::*;
    use crate::application::app_context::AppSnapshot;
    use crate::db::webhooks::WebhookDelivery;
    use crate::ports::types::SubscriptionLookupSnapshot;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn snapshot(id: Uuid, user: &str, token: &str, last_event_time: i64) -> SubscriptionLookupSnapshot {
        SubscriptionLookupSnapshot {
            id,
            external_user_id: user.to_string(),
            provider: "google_play".to_string(),
            subscription_id: "hiha_monthly".to_string(),
            purchase_token: Some(token.to_string()),
            status: "active".to_string(),
            current_period_end: None,
            auto_renewing: Some(true),
            revocation_reason: None,
            last_event_time,
            google_price_step_up_consent_deadline: None,
            google_pause_scheduled_at: None,
            google_deferred_until: None,
            google_pending_price_change_new_price_cents: None,
            google_pending_price_change_currency: None,
            google_pending_price_change_mode: None,
            google_pending_price_change_state: None,
            google_pending_price_change_expected_at: None,
        }
    }

    fn payload(provider: &str, token: Option<&str>, timestamp_epoch_ms: i64) -> CanonicalWebhookPayload {
        CanonicalWebhookPayload {
            event_id: "evt".to_string(),
            event_type: "subscription.cancelled".to_string(),
            timestamp: "2026-06-26T00:00:00Z".to_string(),
            timestamp_epoch_ms,
            app_slug: "hiha".to_string(),
            product_id: Some("hiha_monthly".to_string()),
            subscription_id: Some("hiha_monthly".to_string()),
            external_user_id: Some("user".to_string()),
            amount_cents: None,
            new_price_cents: None,
            auto_renewing: Some(true),
            purchase_token: token.map(str::to_string),
            current_period_end: None,
            status: Some("cancelled".to_string()),
            provider: provider.to_string(),
            provider_event_id: "prov_evt".to_string(),
            previous_status: None,
            corrected_status: None,
            reconciliation_source: None,
            revocation_reason: None,
            cancellation_mode: None,
            google_price_step_up_consent_deadline: None,
            google_pause_scheduled_at: None,
            google_deferred_until: None,
            google_pending_price_change_new_price_cents: None,
            google_pending_price_change_currency: None,
            google_pending_price_change_mode: None,
            google_pending_price_change_state: None,
            google_pending_price_change_expected_at: None,
        }
    }

    fn delivery(app_id: Uuid, id: Uuid) -> WebhookDelivery {
        WebhookDelivery {
            id,
            app_id,
            webhook_provider_id: Uuid::new_v4(),
            forward_attempts: 0,
            forwarded: false,
            forwarded_at: None,
            dead_lettered: false,
            dead_lettered_at: None,
            dead_letter_reason: None,
            last_http_status: None,
            last_error: None,
            canonical_payload: None,
            claim_token: Some(Uuid::nil()),
            claimed_by: Some("test-worker".to_string()),
            claimed_until: Some(Utc::now() + chrono::Duration::minutes(10)),
            next_attempt_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    struct MockRepo {
        app_id: Uuid,
        delivery_id: Uuid,
        /// Row returned by purchase-token lookup (the correct, user-unique row).
        by_token: HashMap<String, SubscriptionLookupSnapshot>,
        /// Row returned by subscription_id lookup (for google_play this would be
        /// the WRONG same-SKU row; the fix must not consult it).
        by_sub_id: Option<SubscriptionLookupSnapshot>,
        suppressed: AtomicBool,
    }

    #[async_trait]
    impl crate::ports::traits::subscription::SubscriptionLookupRepository for MockRepo {
        async fn get_subscription_by_sub_id(
            &self,
            _app_id: Uuid,
            _subscription_id: &str,
        ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
            Ok(self.by_sub_id.clone())
        }

        async fn get_subscription_by_sub_id_and_user(
            &self,
            _app_id: Uuid,
            _subscription_id: &str,
            _external_user_id: &str,
        ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
            unreachable!("not used by forward_webhook")
        }

        async fn get_subscription_by_purchase_token(
            &self,
            _app_id: Uuid,
            purchase_token: &str,
        ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
            Ok(self.by_token.get(purchase_token).cloned())
        }
    }

    #[async_trait]
    impl crate::ports::traits::WebhookSuppressionRepository for MockRepo {
        async fn suppress_webhook(&self, _webhook_id: Uuid, _reason: &str) -> Result<(), BridgeError> {
            self.suppressed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl crate::ports::traits::WebhookForwardRepository for MockRepo {
        async fn get_webhook_delivery(&self, id: Uuid) -> Result<WebhookDelivery, BridgeError> {
            Ok(delivery(self.app_id, id))
        }

        async fn webhook_delivery_exists(&self, _webhook_provider_id: Uuid) -> Result<bool, BridgeError> {
            Ok(true)
        }

        async fn complete_webhook_delivery_attempt(
            &self,
            delivery_id: Uuid,
            _claim_token: Uuid,
            _http_status: Option<i32>,
            _error: Option<String>,
            _forwarded: bool,
        ) -> Result<WebhookDelivery, BridgeError> {
            Ok(delivery(self.app_id, delivery_id))
        }

        async fn refresh_webhook_delivery_claim(
            &self,
            _app_id: Uuid,
            _delivery_id: Uuid,
            _claim_token: Uuid,
            _lease_secs: i64,
        ) -> Result<bool, BridgeError> {
            Ok(true)
        }

        async fn claim_webhook_delivery_by_id(
            &self,
            _app_id: Uuid,
            delivery_id: Uuid,
            _worker_id: &str,
            _lease_secs: i64,
        ) -> Result<Option<WebhookDelivery>, BridgeError> {
            Ok(Some(delivery(self.app_id, delivery_id)))
        }

        async fn reset_webhook_delivery(&self, _delivery_id: Uuid) -> Result<bool, BridgeError> {
            unreachable!("not used by forward_webhook")
        }
    }

    #[async_trait]
    impl crate::ports::traits::AppLookupRepository for MockRepo {
        async fn get_app(&self, _app_id: Uuid) -> Result<AppSnapshot, BridgeError> {
            Ok(AppSnapshot {
                id: self.app_id,
                slug: "hiha".to_string(),
                display_name: "HiHa".to_string(),
                // Closed local port: forward attempts fail fast (connection refused).
                webhook_callback_url: "http://127.0.0.1:9/callback".to_string(),
                webhook_callback_secret: "test_secret".to_string(),
                api_rate_limit_per_minute: 60,
                api_rate_limit_rules: None,
                app_url: None,
                google_package_name: None,
                apple_bundle_id: None,
            })
        }
    }

    fn mock(by_token: Vec<SubscriptionLookupSnapshot>, by_sub_id: Option<SubscriptionLookupSnapshot>) -> MockRepo {
        let mut map = HashMap::new();
        for s in by_token {
            map.insert(s.purchase_token.clone().unwrap(), s);
        }
        MockRepo {
            app_id: Uuid::new_v4(),
            delivery_id: Uuid::new_v4(),
            by_token: map,
            by_sub_id,
            suppressed: AtomicBool::new(false),
        }
    }

    #[tokio::test]
    async fn google_event_not_suppressed_against_other_users_newer_same_sku_row() {
        // The bug: user A's event measured against user B's newer same-SKU row.
        let row_a = snapshot(Uuid::new_v4(), "user_a", "token_a", 1_000);
        let row_b = snapshot(Uuid::new_v4(), "user_b", "token_b", 9_000);
        // sub_id lookup would return the wrong (newer) row B; token lookup -> A.
        let repo = mock(vec![row_a, row_b.clone()], Some(row_b));

        // Event for user A (token_a), newer than A's own row but older than B's.
        let p = payload("google_play", Some("token_a"), 5_000);
        forward_webhook(&repo, repo.app_id, repo.delivery_id, Uuid::nil(), p).await.unwrap();

        assert!(
            !repo.suppressed.load(Ordering::SeqCst),
            "Google event must NOT be suppressed against another user's same-SKU row"
        );
    }

    #[tokio::test]
    async fn google_event_suppressed_against_same_users_own_newer_row() {
        let row_a = snapshot(Uuid::new_v4(), "user_a", "token_a", 9_000);
        let repo = mock(vec![row_a], None);

        // Event for user A, older than A's own row -> genuinely stale.
        let p = payload("google_play", Some("token_a"), 5_000);
        forward_webhook(&repo, repo.app_id, repo.delivery_id, Uuid::nil(), p).await.unwrap();

        assert!(
            repo.suppressed.load(Ordering::SeqCst),
            "genuinely stale Google event must be suppressed via its own purchase-token row"
        );
    }

    #[tokio::test]
    async fn google_event_with_missing_token_fails_open() {
        // No row matches the token -> must forward, never suppress on SKU.
        let row_b = snapshot(Uuid::new_v4(), "user_b", "token_b", 9_000);
        let repo = mock(vec![], Some(row_b));

        let p = payload("google_play", Some("token_unmatched"), 1);
        forward_webhook(&repo, repo.app_id, repo.delivery_id, Uuid::nil(), p).await.unwrap();

        assert!(
            !repo.suppressed.load(Ordering::SeqCst),
            "unmatched Google purchase token must fail open, not suppress on shared SKU"
        );
    }

    #[tokio::test]
    async fn non_google_provider_still_uses_subscription_id_keying() {
        // For non-google providers, subscription_id is user-unique: keep keying on it.
        let mut stored = snapshot(Uuid::new_v4(), "user_c", "token_c", 9_000);
        stored.provider = "creem".to_string();
        let repo = mock(vec![], Some(stored));

        let p = payload("creem", None, 5_000);
        forward_webhook(&repo, repo.app_id, repo.delivery_id, Uuid::nil(), p).await.unwrap();

        assert!(
            repo.suppressed.load(Ordering::SeqCst),
            "stale non-google event must still be suppressed via subscription_id row"
        );
    }
}
