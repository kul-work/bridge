use crate::error::BridgeError;
use chrono::{DateTime, Utc};
use serde_json::Value;
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
    pub fn new(config: CreemConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    /// Create from JSON configuration
    pub fn from_json(config_json: &Value) -> Result<Self, BridgeError> {
        let config = CreemConfig::from_json(config_json)?;
        Ok(Self::new(config))
    }

    /// Create checkout session
    pub async fn create_checkout(
        &self,
        product_id: &str,
        email: &str,
        metadata: serde_json::Value,
        success_url: &str,
        cancel_url: &str,
    ) -> Result<(String, String), BridgeError> {
        let request = CreateCheckoutRequest {
            product_id: product_id.to_string(),
            customer: CustomerData {
                email: email.to_string(),
            },
            metadata,
            success_url: success_url.to_string(),
            cancel_url: cancel_url.to_string(),
        };

        let response = self
            .http
            .post(format!("{}/checkouts", self.config.base_url()))
            .header("x-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Creem checkout failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BridgeError::ProviderError(format!(
                "Creem checkout failed: {} - {}",
                status, body
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
    ) -> Result<(), BridgeError> {
        let request = ModifySubscriptionRequest {
            mode: mode.map(|m| m.to_string()),
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
            error!("Creem cancel failed: {} - {}", status, body);
            return Err(BridgeError::ProviderError(format!(
                "Creem cancel failed: {}",
                status
            )));
        }

        info!("Creem subscription {} cancelled via API", subscription_id);
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
            error!("Creem resume failed: {} - {}", status, body);
            return Err(BridgeError::ProviderError(format!(
                "Creem resume failed: {}",
                status
            )));
        }

        info!("Creem subscription {} resumed via API", subscription_id);
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
            error!("Creem billing portal failed: {} - {}", status, body);
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
    ) -> Result<(String, Option<DateTime<Utc>>), BridgeError> {
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
            error!("Creem get subscription failed: {} - {}", status, body);
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

/// Normalize raw Creem status to canonical form
fn normalize_creem_status(raw: &str) -> String {
    match raw {
        "trialing" => "trial".to_string(),
        "active" | "paid" => "active".to_string(),
        "past_due" | "unpaid" => "past_due".to_string(),
        "canceled" | "cancelled" => "cancelled".to_string(),
        "expired" => "expired".to_string(),
        "paused" => "paused".to_string(),
        other => other.to_string(),
    }
}
