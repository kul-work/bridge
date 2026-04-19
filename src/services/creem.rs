use async_trait::async_trait;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use crate::error::AppError;
use crate::services::payment::{
    CheckoutSession, PaymentProvider, ProviderData, SubscriptionDetails, SubscriptionStatus,
    WebhookEvent,
};

type HmacSha256 = Hmac<Sha256>;

/// Creem payment provider
/// Archived: Not currently instantiated. Use if Creem integration is needed in future.
#[allow(dead_code)]
pub struct CreemProvider {
    api_key: String,
    product_id: String,
    offer_id: String,
    otp_id: String,
    webhook_secret: String,
    api_url: String,
    base_url: String,
    mock_external_apis: bool,
    client: reqwest::Client,
}

impl CreemProvider {
    #[allow(dead_code)]
    pub fn new(
        api_key: String,
        product_id: String,
        offer_id: String,
        otp_id: String,
        webhook_secret: String,
        api_url: String,
        base_url: String,
        mock_external_apis: bool,
    ) -> Self {
        Self {
            api_key,
            product_id,
            offer_id,
            otp_id,
            webhook_secret,
            api_url,
            base_url,
            mock_external_apis,
            client: reqwest::Client::new(),
        }
    }

    #[allow(dead_code)]
    fn normalize_event_type(raw_event_type: &str, billing_type: Option<&str>) -> String {
        match raw_event_type {
            // checkout.completed may represent subscription checkout completion when billing_type is recurring or monthly
            "checkout.completed" => {
                if billing_type == Some("recurring") || billing_type == Some("monthly") {
                    "subscription.created".to_string()
                } else {
                    "one_time_product.purchased".to_string()
                }
            }
            "subscription.active" | "subscription.trialing" => "subscription.created".to_string(),
            "subscription.past_due" => "subscription.grace_period".to_string(),
            "subscription.scheduled_cancel" => "subscription.cancellation_scheduled".to_string(),
            "subscription.canceled" => "subscription.cancelled".to_string(),
            other => other.to_string(),
        }
    }

    #[allow(dead_code)]
    fn normalize_status(raw_status: Option<&str>) -> String {
        match raw_status.unwrap_or("unknown") {
            // Internal status values expected by DB and handlers
            "trialing" => "trial".to_string(),
            "paid" => "active".to_string(),
            "unpaid" => "past_due".to_string(),
            "canceled" => "cancelled".to_string(),
            other => other.to_string(),
        }
    }

    #[allow(dead_code)]
    async fn map_creem_api_error(
        response: reqwest::Response,
        operation: &str,
    ) -> AppError {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!(
            "Creem API {} failed: status={}, body={}",
            operation,
            status,
            body
        );
        AppError::PaymentProviderError(format!("{} failed", operation))
    }
}

#[async_trait]
impl PaymentProvider for CreemProvider {
    async fn create_checkout(
        &self,
        user_id: &str,
        email: &str,
        product_type: Option<&str>,
    ) -> Result<CheckoutSession, AppError> {
        if self.mock_external_apis {
            tracing::info!("MOCK: Skipping Creem create_checkout (mock mode)");
            return Ok(CheckoutSession {
                redirect_url: format!("{}/story?mock=true", self.base_url),
                session_id: format!("mock_creem_{}", Utc::now().timestamp()),
            });
        }

        let client = &self.client;

        let success_url = format!("{}/story", self.base_url);

        let selected_product_id = match product_type {
            Some("offer") => &self.offer_id,
            Some("otp") => &self.otp_id,
            _ => &self.product_id,
        };

        let payload = serde_json::json!({
            "product_id": selected_product_id,
            "customer": {
                "email": email
            },
            "metadata": {
                "user_id": user_id
            },
            "success_url": success_url
        });

        let response = client
            .post(format!("{}/checkouts", self.api_url.trim_end_matches('/')))
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to create checkout: {}", e)))?;

        if !response.status().is_success() {
            return Err(Self::map_creem_api_error(response, "Create checkout").await);
        }

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Invalid API response: {}", e)))?;

        tracing::debug!("Creem API (not-webhook) response: {}", serde_json::to_string_pretty(&data).unwrap_or_default());

        let checkout_url = data["checkout_url"]
            .as_str()
            .ok_or_else(|| AppError::PaymentProviderError("Missing checkout URL".to_string()))?;

        let session_id = data["id"]
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
        // Verify HMAC signature (no prefix for Creem)
        let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
            .map_err(|_| AppError::WebhookVerificationFailed)?;

        mac.update(body);

        // Constant-time comparison to prevent timing attacks
        let sig_bytes = hex::decode(signature)
            .map_err(|_| AppError::WebhookVerificationFailed)?;
        mac.verify_slice(&sig_bytes)
            .map_err(|_| {
                tracing::error!("Creem webhook signature verification failed - signatures do not match");
                AppError::WebhookVerificationFailed
            })?;
        
        // tracing::debug!("Creem signature verification SUCCESS");

        // Parse webhook payload
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| {
                tracing::error!("Failed to parse Creem webhook payload: {}", e);
                AppError::WebhookVerificationFailed
            })?;

        tracing::debug!("Creem webhook payload: {}", serde_json::to_string_pretty(&payload).unwrap_or_default());
        
        let raw_product_id = payload["object"]["product"]["id"].as_str();
        tracing::debug!("DEBUG: raw product_id in payload: {:?}", raw_product_id);

        // Extract event ID for idempotency (top-level "id" field)
        let event_id = payload["id"]
            .as_str()
            .map(|s| s.to_string());

        // Extract event type from Creem webhook format
        // Creem uses "eventType" not "event_type"
        let raw_event_type = payload["eventType"]
            .as_str()
            .ok_or_else(|| {
                tracing::error!("Missing eventType in Creem webhook");
                AppError::WebhookVerificationFailed
            })?;

        // Extract subscription data from "object" field
        let obj = &payload["object"];

        let object_id = obj["id"]
            .as_str()
            .map(|s| s.to_string());
        let object_order_id = obj["order_id"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| obj["order"]["id"].as_str().map(|s| s.to_string()));

        let object_checkout_id = obj["checkout_id"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| obj["checkout"]["id"].as_str().map(|s| s.to_string()));
        
        // Multi-level extraction with fallbacks for subscription_id
        let object_subscription_id = obj["subscription_id"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| obj["subscription"]["id"].as_str().map(|s| s.to_string()));
        
        // Multi-level extraction with fallbacks for product_id
        let _object_product_id = obj["product_id"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| obj["product"]["id"].as_str().map(|s| s.to_string()))
            .or_else(|| obj["checkout"]["product"].as_str().map(|s| s.to_string()));
        
        // Multi-level extraction with fallbacks for billing_type
        let billing_type = obj["billing_type"]
            .as_str()
            .or_else(|| obj["product"]["billing_type"].as_str())
            .or_else(|| obj["order"]["type"].as_str())
            .or_else(|| obj["subscription"].get("product").and_then(|p| p["billing_type"].as_str()));

        // Choose the right subscription_id: prefer subscription_id > order_id > checkout_id > object_id
        let subscription_id = object_subscription_id.clone()
            .or_else(|| object_order_id.clone())
            .or_else(|| object_checkout_id.clone())
            .or(object_id.clone());

        // Extract email from customer object (multi-level with fallback)
        let customer_email = obj["customer"]["email"]
            .as_str()
            .or_else(|| obj["customer"].as_str())
            .or_else(|| obj["email"].as_str())
            .unwrap_or("unknown@example.com")
            .to_string();

        // Extract status
        let raw_status = obj["status"].as_str();
        let status = Self::normalize_status(raw_status);

        // Normalize event type
        let event_type = Self::normalize_event_type(raw_event_type, billing_type);

        // Extract renewal date (renews_at)
        let current_period_end = if let Some(date_str) = obj["renews_at"].as_str() {
            DateTime::parse_from_rfc3339(date_str)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        } else {
            None
        };

        // Extract amount in cents from price (if available) or total (if available)
        let amount_cents = obj["price"]
            .as_i64()
            .map(|a| a as i32)
            .or_else(|| obj["total"].as_i64().map(|a| a as i32));

        // Extract customer ID (provider_customer_id)
        let provider_customer_id = obj["customer"]["id"]
            .as_str()
            .map(|s| s.to_string());

        // Extract metadata.user_id (may not exist for all webhooks)
        let metadata_user_id = obj["metadata"]["user_id"]
            .as_str()
            .map(|s| s.to_string());

        // Extract purchase_token and provider_transaction_id
        let purchase_token = object_subscription_id;
        let provider_transaction_id = obj["last_transaction_id"]
            .as_str()
            .map(|s| s.to_string());

        // Extract auto_renewing if available
        let auto_renewing = if let Some(v) = obj["auto_renew"].as_bool() {
            Some(v)
        } else {
            None
        };

        // Warn if metadata.user_id is missing (orphaned payment)
        if metadata_user_id.is_none() && raw_event_type.contains("checkout") {
            tracing::warn!(
                raw_event_type = %raw_event_type,
                event_id = ?event_id,
                subscription_id = ?subscription_id,
                customer_email = %customer_email,
                "Orphaned payment detected: missing metadata.user_id (admin review required)"
            );
        }

        tracing::info!("Creem webhook parsed SUCCESS: event_type={}, subscription_id={:?}, email={}, status={}, purchase_token={:?}", 
               event_type, subscription_id, customer_email, status, purchase_token);

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
            metadata_user_id,
            purchase_token,
            payment_state: None,
            cancel_reason: None,
            auto_renewing,
            subscription_state: None,
            grace_period_expiration: None,
            deferred_until: None,
            obfuscated_account_id: None,
            provider_transaction_id,
        })
    }

    async fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<SubscriptionDetails, AppError> {
        if self.mock_external_apis {
            tracing::info!("MOCK: Skipping Creem get_subscription (mock mode)");
            return Ok(SubscriptionDetails {
                subscription_id: subscription_id.to_string(),
                status: SubscriptionStatus::Active,
                customer_email: "mock-user@example.com".to_string(),
                current_period_end: Some(Utc::now() + chrono::Duration::days(30)),
                purchase_token: None,
                payment_state: None,
                cancel_reason: None,
                auto_renewing: Some(true),
                amount_cents: Some(2999), 
                acknowledged_at: None,
                provider_data: ProviderData::None,
            });
        }

        let client = &self.client;

        let response = client
            .get(format!(
                "{}/subscriptions/{}",
                self.api_url.trim_end_matches('/'),
                subscription_id
            ))
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to fetch subscription: {}", e)))?;

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Invalid API response: {}", e)))?;

        let status = data["status"]
            .as_str()
            .ok_or(AppError::SubscriptionNotFound)?;

        let customer_email = data["customer"]["email"]
            .as_str()
            .ok_or(AppError::SubscriptionNotFound)?
            .to_string();

        let current_period_end = if let Some(date_str) = data["renews_at"].as_str() {
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
        self.cancel_subscription_with_mode(subscription_id, None, None).await
    }

    async fn cancel_subscription_with_mode(
        &self,
        subscription_id: &str,
        mode: Option<&str>,
        on_execute: Option<&str>,
    ) -> Result<(), AppError> {
        if self.mock_external_apis {
            tracing::info!("MOCK: Skipping Creem cancel_subscription (mock mode)");
            return Ok(());
        }

        let client = &self.client;

        let mut payload = serde_json::Map::new();
        if let Some(mode) = mode {
            payload.insert("mode".to_string(), serde_json::Value::String(mode.to_string()));
        }
        if let Some(on_execute) = on_execute {
            payload.insert("onExecute".to_string(), serde_json::Value::String(on_execute.to_string()));
        }

        let response = client
            .post(format!(
                "{}/subscriptions/{}/cancel",
                self.api_url.trim_end_matches('/'),
                subscription_id
            ))
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::Value::Object(payload))
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to cancel subscription: {}", e)))?;

        if !response.status().is_success() {
            return Err(Self::map_creem_api_error(response, "Cancel subscription").await);
        }

        Ok(())
    }

    async fn create_billing_portal(&self, customer_id: &str) -> Result<String, AppError> {
        if self.mock_external_apis {
            tracing::info!("MOCK: Skipping Creem create_billing_portal (mock mode)");
            return Ok(format!("{}/billing-portal-mock", self.base_url));
        }

        let client = &self.client;
        let response = client
            .post(format!("{}/customers/billing", self.api_url.trim_end_matches('/')))
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "customer_id": customer_id }))
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to create billing portal: {}", e)))?;

        if !response.status().is_success() {
            return Err(Self::map_creem_api_error(response, "Create billing portal").await);
        }

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|_| AppError::PaymentProviderError("Create billing portal failed".to_string()))?;

        // Check for `url` field (primary expected key per Creem API)
        let url = data["url"]
            .as_str()
            .ok_or_else(|| {
                tracing::error!("Creem billing portal response missing 'url' field: {:?}", data);
                AppError::PaymentProviderError("Invalid billing portal response from provider".to_string())
            })?;

        Ok(url.to_string())
    }

    async fn resume_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<(), AppError> {
        if self.mock_external_apis {
            tracing::info!("MOCK: Skipping Creem resume_subscription (mock mode)");
            return Ok(());
        }

        let client = &self.client;
        let response = client
            .post(format!(
                "{}/subscriptions/{}/resume",
                self.api_url.trim_end_matches('/'),
                subscription_id
            ))
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(format!("Failed to resume subscription: {}", e)))?;

        if !response.status().is_success() {
            return Err(Self::map_creem_api_error(response, "Resume subscription").await);
        }

        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "creem"
    }

    fn signature_header_name(&self) -> &'static str {
        "creem-signature"
    }

    fn webhook_id_header_name(&self) -> &'static str {
        "x-not-used"    // Creem provides event ID in payload, not in headers
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
        // Creem does not require explicit acknowledgment of purchases
        tracing::debug!(
            "acknowledge_purchase_idempotent called for Creem (no-op): subscription_id={}",
            subscription_id
        );
        Ok(())
    }
}
