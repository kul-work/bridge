use async_trait::async_trait;
use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use crate::error::AppError;
use crate::services::payment::{
    CheckoutSession, PaymentProvider, SubscriptionDetails,
    WebhookEvent,
};

type HmacSha256 = Hmac<Sha256>;

/// Coinbase charge details (status and amount from a verification fetch)
/// Used for 402 micropayment verification.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CoinbaseChargeDetails {
    pub status: String,
    pub amount_cents: i32,
}

/// Coinbase Commerce provider for 402 micropayments
/// Archived: Not currently instantiated. Use if Coinbase 402 integration is needed in future.
#[allow(dead_code)]
pub struct CoinbaseProvider {
    api_key: String,
    webhook_secret: String,
    mock_external_apis: bool,
    client: reqwest::Client,
}

impl CoinbaseProvider {
    #[allow(dead_code)]
    pub fn new(
        api_key: String,
        webhook_secret: String,
        mock_external_apis: bool,
    ) -> Self {
        Self {
            api_key,
            webhook_secret,
            mock_external_apis,
            client: reqwest::Client::new(),
        }
    }

    /// Fetch charge details (status and amount) in a single API call
    #[allow(dead_code)]
    pub async fn get_charge_details(&self, charge_id: &str) -> Result<CoinbaseChargeDetails, AppError> {
        if self.mock_external_apis {
            // Only confirm charges created by our mock (prefix mock_charge_)
            if charge_id.starts_with("mock_charge_") {
                return Ok(CoinbaseChargeDetails {
                    status: "COMPLETED".to_string(),
                    amount_cents: 100, // 100 cents = $1
                });
            }
            return Err(AppError::PaymentProviderError(
                format!("Coinbase charge {} not found", charge_id)
            ));
        }

        let response = self.client
            .get(format!("https://api.commerce.coinbase.com/charges/{}", charge_id))
            .header("X-CC-Api-Key", &self.api_key)
            .header("X-CC-Version", "2018-03-22")
            .send()
            .await
            .map_err(|e| AppError::PaymentProviderError(
                format!("Coinbase charge fetch failed: {}", e)
            ))?;

        if !response.status().is_success() {
            return Err(AppError::PaymentProviderError(
                format!("Coinbase charge {} not found", charge_id)
            ));
        }

        let body: serde_json::Value = response.json().await
            .map_err(|e| AppError::PaymentProviderError(
                format!("Failed to parse Coinbase response: {}", e)
            ))?;

        // Status extraction (checking multiple fields for robustness)
        let status = body["data"]["status"].as_str()
            .or_else(|| {
                body["data"]["timeline"]
                    .as_array()
                    .and_then(|a| a.last())
                    .and_then(|last| last["status"].as_str())
            })
            .unwrap_or("UNKNOWN")
            .to_uppercase();

        // Fix extraction: Sum confirmed payments for accurate validation
        let mut amount_cents = 0;
        if let Some(payments) = body["data"]["payments"].as_array() {
            for payment in payments {
                if payment["status"].as_str() == Some("CONFIRMED") {
                    let val_str = payment["value"]["local"]["amount"].as_str().unwrap_or("0");
                    amount_cents += (val_str.parse::<f64>().unwrap_or(0.0) * 100.0).round() as i32;
                }
            }
        }

        Ok(CoinbaseChargeDetails { status, amount_cents })
    }

    /// Normalize Coinbase webhook event type to our internal format
    #[allow(dead_code)]
    fn normalize_event_type(raw: &str) -> String {
        match raw {
            "charge:confirmed" => "charge.confirmed".to_string(),
            "charge:failed" => "charge.failed".to_string(),
            "charge:pending" => "charge.pending".to_string(),
            "charge:resolved" => "charge.confirmed".to_string(),
            _ => raw.replace(':', "."),
        }
    }
}

#[async_trait]
impl PaymentProvider for CoinbaseProvider {
    // Coinbase uses credit-based model, not subscription checkout
    async fn create_checkout(
        &self,
        _user_id: &str,
        _email: &str,
        _product_type: Option<&str>,
    ) -> Result<CheckoutSession, AppError> {
        Err(AppError::PaymentProviderError(
            "Coinbase does not support subscription checkout. Use 402 payment flow.".to_string()
        ))
    }

    async fn verify_and_parse_webhook(
        &self,
        body: &[u8],
        signature: &str,
        _headers: &HeaderMap,
    ) -> Result<WebhookEvent, AppError> {
        // HMAC-SHA256 verification
        if !self.mock_external_apis {
            let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
                .map_err(|e| AppError::WebhookSignatureVerificationFailed(
                    format!("HMAC init failed: {}", e)
                ))?;
            mac.update(body);
            let expected = hex::decode(signature)
                .map_err(|e| AppError::WebhookSignatureVerificationFailed(
                    format!("Invalid hex signature: {}", e)
                ))?;
            mac.verify_slice(&expected)
                .map_err(|_| AppError::WebhookSignatureVerificationFailed(
                    "Coinbase webhook signature mismatch".to_string()
                ))?;
        }

        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| AppError::WebhookPayloadParsingFailed(
                format!("Failed to parse Coinbase webhook: {}", e)
            ))?;

        let event_data = &payload["event"];
        let raw_type = event_data["type"].as_str()
            .ok_or_else(|| AppError::WebhookPayloadInvalid("Missing event type".to_string()))?;
        let event_type = Self::normalize_event_type(raw_type);

        let charge_data = &event_data["data"];
        let charge_id = charge_data["id"].as_str()
            .unwrap_or("")
            .to_string();
        let event_id = event_data["id"].as_str()
            .map(|s| s.to_string());

        // Extract metadata (email, amount_cents, request_type)
        let metadata = &charge_data["metadata"];
        let email = metadata["email"].as_str()
            .unwrap_or("")
            .to_string();
        let amount_cents = metadata["amount_cents"].as_i64()
            .unwrap_or(0) as i32;

        Ok(WebhookEvent {
            event_type,
            event_id,
            event_time_millis: None,
            subscription_id: Some(charge_id.clone()),
            customer_email: email,
            purchase_token: Some(charge_id),
            provider_customer_id: None,
            status: "completed".to_string(),
            current_period_end: None,
            amount_cents: Some(amount_cents),
            metadata_user_id: None,
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
        _subscription_id: &str,
    ) -> Result<SubscriptionDetails, AppError> {
        Err(AppError::PaymentProviderError(
            "Coinbase does not support subscriptions".to_string()
        ))
    }

    async fn cancel_subscription(
        &self,
        _subscription_id: &str,
    ) -> Result<(), AppError> {
        Err(AppError::PaymentProviderError(
            "Coinbase does not support subscriptions".to_string()
        ))
    }

    fn provider_name(&self) -> &'static str {
        "coinbase"
    }

    fn signature_header_name(&self) -> &'static str {
        "x-cc-webhook-signature"
    }

    fn webhook_id_header_name(&self) -> &'static str {
        "x-cc-webhook-id"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn acknowledge_purchase_idempotent(
        &self,
        _subscription_id: &str,
        _purchase_token: &str,
        _purchase_type: crate::services::payment::PurchaseType,
        _user_id: Option<&str>,
    ) -> Result<(), AppError> {
        // Coinbase charges are self-acknowledging
        Ok(())
    }
}
