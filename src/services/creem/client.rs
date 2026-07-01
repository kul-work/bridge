use crate::error::BridgeError;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::time::Duration;
use tracing::{error, info};

use super::config::CreemConfig;
use super::models::*;

/// Creem API client
pub struct CreemClient {
    http: reqwest::Client,
    config: CreemConfig,
}

impl CreemClient {
    /// Create new Creem client from config
    pub fn new(config: CreemConfig) -> Result<Self, BridgeError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|e| BridgeError::InternalServerError(format!("Failed to build Creem HTTP client: {}", e)))?;

        Ok(Self { http, config })
    }

    /// Create from JSON configuration
    pub fn from_json(config_json: &Value) -> Result<Self, BridgeError> {
        let config = CreemConfig::from_json(config_json)?;
        Self::new(config)
    }

    /// Create checkout session
    pub async fn create_checkout(
        &self,
        product_id: &str,
        email: &str,
        metadata: serde_json::Value,
        success_url: &str,
    ) -> Result<(String, String), BridgeError> {
        let request = CreateCheckoutRequest {
            product_id: product_id.to_string(),
            customer: CustomerData {
                email: email.to_string(),
            },
            metadata,
            success_url: success_url.to_string(),
        };

        let response = self
            .http
            .post(format!("{}/checkouts", self.config.base_url()))
            .header("x-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| BridgeError::ProviderError(format!(
                "Creem checkout failed for product_id '{}': {}",
                product_id, e
            )))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let scrubbed_body = scrub_creem_error_body(&body);
            error!(
                provider = "creem",
                operation = "create_checkout",
                product_id,
                status = status.as_u16(),
                error_body = %scrubbed_body,
                "Creem checkout failed"
            );
            return Err(BridgeError::ProviderError(format!(
                "Creem checkout failed for product_id '{}': {}",
                product_id, status
            )));
        }

        let data: CreateCheckoutResponse = response
            .json()
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Invalid Creem response: {}", e)))?;

        let checkout_url = data
            .get_checkout_url()
            .ok_or_else(|| BridgeError::ProviderError("Missing Creem checkout_url".to_string()))?
            .to_string();

        let session_id = data
            .get_session_id()
            .unwrap_or("")
            .to_string();

        Ok((session_id, checkout_url))
    }

    /// Cancel subscription
    pub async fn cancel_subscription(
        &self,
        subscription_id: &str,
        mode: Option<&str>,
        on_execute: Option<&str>,
    ) -> Result<(), BridgeError> {
        let request = ModifySubscriptionRequest {
            mode: mode.map(|m| m.to_string()),
            on_execute: on_execute.map(|o| o.to_string()),
        };

        let response = self
            .http
            .post(format!(
                "{}/subscriptions/{}/cancel",
                self.config.base_url(),
                subscription_id
            ))
            .header("x-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Creem cancel failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let scrubbed_body = scrub_creem_error_body(&body);
            error!(
                provider = "creem",
                operation = "cancel_subscription",
                subscription_id,
                status = status.as_u16(),
                error_body = %scrubbed_body,
                "Creem cancel failed"
            );
            return Err(BridgeError::ProviderError(format!(
                "Creem cancel failed: {}",
                status
            )));
        }

        info!(
            provider = "creem",
            operation = "cancel_subscription",
            subscription_id,
            "Creem subscription cancelled via API"
        );
        Ok(())
    }

    /// Resume subscription
    pub async fn resume_subscription(&self, subscription_id: &str) -> Result<(), BridgeError> {
        let response = self
            .http
            .post(format!(
                "{}/subscriptions/{}/resume",
                self.config.base_url(),
                subscription_id
            ))
            .header("x-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Creem resume failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let scrubbed_body = scrub_creem_error_body(&body);
            error!(
                provider = "creem",
                operation = "resume_subscription",
                subscription_id,
                status = status.as_u16(),
                error_body = %scrubbed_body,
                "Creem resume failed"
            );
            return Err(BridgeError::ProviderError(format!(
                "Creem resume failed: {}",
                status
            )));
        }

        info!(
            provider = "creem",
            operation = "resume_subscription",
            subscription_id,
            "Creem subscription resumed via API"
        );
        Ok(())
    }

    /// Create billing portal URL
    pub async fn create_billing_portal(
        &self,
        customer_id: &str,
    ) -> Result<String, BridgeError> {
        let response = self
            .http
            .post(format!("{}/customers/billing", self.config.base_url()))
            .header("x-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "customer_id": customer_id }))
            .send()
            .await
            .map_err(|e| {
                BridgeError::ProviderError(format!("Creem billing portal failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let scrubbed_body = scrub_creem_error_body(&body);
            error!(
                provider = "creem",
                operation = "create_billing_portal",
                status = status.as_u16(),
                error_body = %scrubbed_body,
                "Creem billing portal failed"
            );
            return Err(BridgeError::ProviderError(format!(
                "Creem billing portal failed: {}",
                status
            )));
        }

        let data: BillingPortalResponse = response.json().await.map_err(|e| {
            BridgeError::ProviderError(format!("Invalid billing portal response: {}", e))
        })?;

        Ok(data.url)
    }

    /// Fetch subscription status
    pub async fn fetch_subscription_status(
        &self,
        subscription_id: &str,
    ) -> Result<(Option<String>, Option<DateTime<Utc>>), BridgeError> {
        let response = self
            .http
            .get(format!(
                "{}/subscriptions/{}",
                self.config.base_url(),
                subscription_id
            ))
            .header("x-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| {
                BridgeError::ProviderError(format!("Creem get subscription failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let scrubbed_body = scrub_creem_error_body(&body);
            error!(
                provider = "creem",
                operation = "fetch_subscription_status",
                subscription_id,
                status = status.as_u16(),
                error_body = %scrubbed_body,
                "Creem get subscription failed"
            );
            return Err(BridgeError::ProviderError(format!(
                "Creem get subscription failed: {}",
                status
            )));
        }

        let data: SubscriptionStatusResponse = response.json().await.map_err(|e| {
            BridgeError::ProviderError(format!("Invalid Creem response: {}", e))
        })?;

        let status = normalize_creem_status(&data.status);
        let period_end = data
            .renews_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok((status, period_end))
    }
}

/// Normalize raw Creem status to canonical form.
///
/// Returns `None` for unknown statuses so the reconciliation caller can skip
/// the write instead of persisting a value that violates the
/// `subscriptions_status_check` constraint. Keep this vocabulary in sync with
/// `normalize_status` in the webhook path and the DB CHECK constraint.
fn normalize_creem_status(raw: &str) -> Option<String> {
    match raw {
        "trialing" => Some("trial".to_string()),
        "active" | "paid" => Some("active".to_string()),
        "past_due" | "unpaid" => Some("past_due".to_string()),
        "canceled" | "cancelled" => Some("cancelled".to_string()),
        "expired" => Some("expired".to_string()),
        "paused" => Some("paused".to_string()),
        other => {
            tracing::warn!(raw_status = other, "unknown Creem subscription status ignored");
            None
        }
    }
}

fn scrub_creem_error_body(body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = val.get("message").and_then(|m| m.as_str()) {
            return crate::utils::scrub_email(msg);
        }
        if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
            return crate::utils::scrub_email(err);
        }
        if let Some(err) = val.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
            return crate::utils::scrub_email(err);
        }
    }
    crate::utils::scrub_email(body)
}
