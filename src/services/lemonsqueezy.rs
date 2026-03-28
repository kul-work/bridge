use async_trait::async_trait;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use crate::error::AppError;
use crate::services::payment::{
    CheckoutSession, PaymentProvider, ProviderData, SubscriptionDetails, SubscriptionStatus,
    WebhookEvent,
};

#[allow(dead_code)]
type HmacSha256 = Hmac<Sha256>;

/// Lemon Squeezy payment provider
/// Archived: Not currently instantiated. Use if LemonSqueezy integration is needed in future.
#[allow(dead_code)]
pub struct LemonSqueezyProvider {
    api_key: String,
    product_id: String,
    webhook_secret: String,
    mock_external_apis: bool,
}

impl LemonSqueezyProvider {
    #[allow(dead_code)]
    pub fn new(api_key: String, product_id: String, webhook_secret: String, mock_external_apis: bool) -> Self {
        Self {
            api_key,
            product_id,
            webhook_secret,
            mock_external_apis,
        }
    }
}

#[async_trait]
impl PaymentProvider for LemonSqueezyProvider {
    async fn create_checkout(
        &self,
        _user_id: &str,
        email: &str,
        _product_type: Option<&str>,
    ) -> Result<CheckoutSession, AppError> {
        if self.mock_external_apis {
            tracing::info!("MOCK: Skipping LemonSqueezy create_checkout (mock mode)");
            return Ok(CheckoutSession {
                redirect_url: "https://mock.lemonsqueezy.com/checkout".to_string(),
                session_id: format!("mock_ls_{}", Utc::now().timestamp()),
            });
        }

        // Call Lemon Squeezy API to create checkout
        let client = reqwest::Client::new();

        let product_id = self.product_id.parse::<i32>()
            .map_err(|e| AppError::PaymentProviderError(
                format!("Invalid product ID '{}': {}", self.product_id, e)
            ))?;

        let payload = json!({
            "data": {
                "type": "checkouts",
                "attributes": {
                    "product_id": product_id,
                    "checkout_data": {
                        "email": email
                    }
                }
            }
        });

        let response = client
            .post("https://api.lemonsqueezy.com/v1/checkouts")
            .bearer_auth(&self.api_key)
            .header("Accept", "application/vnd.api+json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to create checkout: {}", e)))?;

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Invalid API response: {}", e)))?;

        let checkout_url = data["data"]["attributes"]["url"]
            .as_str()
            .ok_or_else(|| AppError::PaymentProviderError("Missing checkout URL".to_string()))?;

        let session_id = data["data"]["id"]
            .as_str()
            .ok_or_else(|| AppError::PaymentProviderError("Missing session ID".to_string()))?;

        Ok(CheckoutSession {
            redirect_url: checkout_url.to_string(),
            session_id: session_id.to_string(),
        })
    }

    async fn verify_and_parse_webhook(
        &self,
        body: &[u8],
        signature: &str,
        _headers: &HeaderMap,
    ) -> Result<WebhookEvent, AppError> {
        // Verify HMAC signature
        let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
            .map_err(|_| AppError::WebhookVerificationFailed)?;

        mac.update(body);

        // LemonSqueezy sends signature as "sha256=<hex>"
        let sig_hex = signature
            .strip_prefix("sha256=")
            .ok_or(AppError::WebhookVerificationFailed)?;

        // Constant-time comparison to prevent timing attacks
        let sig_bytes = hex::decode(sig_hex)
            .map_err(|_| AppError::WebhookVerificationFailed)?;
        mac.verify_slice(&sig_bytes)
            .map_err(|_| {
                tracing::error!("Webhook signature verification failed");
                AppError::WebhookVerificationFailed
            })?;

        // Parse webhook payload
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|_| AppError::WebhookVerificationFailed)?;

        // Extract event ID for idempotency
        let event_id = payload["meta"]["webhook_id"]
            .as_str()
            .map(|s| s.to_string());

        let event_type = payload["meta"]["event_name"]
            .as_str()
            .ok_or(AppError::WebhookVerificationFailed)?;

        let data = &payload["data"]["attributes"];

        let subscription_id = if event_type.contains("subscription") {
            payload["data"]["id"].as_str().map(|s| s.to_string())
        } else {
            data["subscription_id"].as_str().map(|s| s.to_string())
        };

        let customer_email = data["customer_email"]
            .as_str()
            .ok_or(AppError::WebhookVerificationFailed)?
            .to_string();

        let provider_customer_id = data["customer_id"]
            .as_str()
            .map(|s| s.to_string());

        let status = data["status"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        let current_period_end = if let Some(date_str) = data["renews_at"].as_str() {
            DateTime::parse_from_rfc3339(date_str)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        } else {
            None
        };

        // Extract amount from total cents
        let amount_cents = data["total_cents"]
            .as_i64()
            .map(|a| a as i32);

        Ok(WebhookEvent {
            event_id,
            event_time_millis: Some(Utc::now().timestamp_millis()),
            event_type: event_type.to_string(),
            subscription_id,
            customer_email,
            status,
            current_period_end,
            amount_cents,
            provider_customer_id,
            metadata_user_id: None,
            purchase_token: None,
            payment_state: None,
            cancel_reason: None,
            auto_renewing: None,
            subscription_state: None,
            grace_period_expiration: None,
            deferred_until: None,
            obfuscated_account_id: None,
            provider_transaction_id: None,
        })
    }

    async fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<SubscriptionDetails, AppError> {
        if self.mock_external_apis {
            tracing::info!("MOCK: Skipping LemonSqueezy get_subscription (mock mode)");
            return Ok(SubscriptionDetails {
                subscription_id: subscription_id.to_string(),
                status: SubscriptionStatus::Active,
                customer_email: "mock-user@example.com".to_string(),
                current_period_end: Some(Utc::now() + chrono::Duration::days(30)),
                purchase_token: None,
                payment_state: None,
                cancel_reason: None,
                auto_renewing: Some(true),
                amount_cents: Some(1999),
                acknowledged_at: None,
                provider_data: ProviderData::None,
            });
        }

        let client = reqwest::Client::new();

        let response = client
            .get(format!(
                "https://api.lemonsqueezy.com/v1/subscriptions/{}",
                subscription_id
            ))
            .bearer_auth(&self.api_key)
            .header("Accept", "application/vnd.api+json")
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to fetch subscription: {}", e)))?;

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Invalid API response: {}", e)))?;

        let attrs = &data["data"]["attributes"];

        let status = attrs["status"]
            .as_str()
            .ok_or(AppError::SubscriptionNotFound)?;

        let customer_email = attrs["customer_email"]
            .as_str()
            .ok_or(AppError::SubscriptionNotFound)?
            .to_string();

        let current_period_end = if let Some(date_str) = attrs["renews_at"].as_str() {
            DateTime::parse_from_rfc3339(date_str)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        } else {
            None
        };

        Ok(SubscriptionDetails {
            subscription_id: subscription_id.to_string(),
            status: SubscriptionStatus::from(status),
            customer_email,
            current_period_end,
            purchase_token: None,
            payment_state: None,
            cancel_reason: None,
            auto_renewing: None,
            amount_cents: None,
            acknowledged_at: None,
            provider_data: ProviderData::None,
        })
    }

    async fn cancel_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<(), AppError> {
        if self.mock_external_apis {
            tracing::info!("MOCK: Skipping LemonSqueezy cancel_subscription (mock mode)");
            return Ok(());
        }

        let client = reqwest::Client::new();

        let payload = json!({
            "data": {
                "type": "subscriptions",
                "id": subscription_id,
                "attributes": {
                    "cancelled": true
                }
            }
        });

        client
            .patch(format!(
                "https://api.lemonsqueezy.com/v1/subscriptions/{}",
                subscription_id
            ))
            .bearer_auth(&self.api_key)
            .header("Accept", "application/vnd.api+json")
            .header("Content-Type", "application/vnd.api+json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to cancel subscription: {}", e)))?;

        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "lemonsqueezy"
    }

    fn signature_header_name(&self) -> &'static str {
        "x-signature"
    }

    fn webhook_id_header_name(&self) -> &'static str {
        "x-webhook-id"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn acknowledge_purchase_idempotent(
        &self,
        subscription_id: &str,
        _purchase_token: &str,
        _purchase_type: crate::services::payment::PurchaseType,
        _user_id: Option<&str>,
    ) -> Result<(), AppError> {
        // LemonSqueezy does not require explicit acknowledgment of purchases
        tracing::debug!(
            "acknowledge_purchase_idempotent called for LemonSqueezy (no-op): subscription_id={}",
            subscription_id
        );
        Ok(())
    }
}
