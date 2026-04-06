use crate::error::BridgeError;
use serde_json::Value;
use tracing::{info, error};

/// Extract a config string field or return ConfigError
fn config_str<'a>(config: &'a Value, key: &str, provider: &str) -> Result<&'a str, BridgeError> {
    config.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError(format!("Missing {} {} config", provider, key)))
}

/// Cancel subscription with the payment provider
pub async fn cancel_subscription(
    provider: &str,
    subscription_id: &str,
    purchase_token: Option<&str>,
    mode: Option<&str>,
    config: &Value,
) -> Result<(), BridgeError> {
    let client = reqwest::Client::new();

    match provider {
        "creem" => {
            let api_key = config_str(config, "api_key", "Creem")?;
            let api_url = config.get("api_url")
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.creem.com");

            let payload = if let Some(mode) = mode {
                serde_json::json!({ "mode": mode })
            } else {
                serde_json::json!({})
            };

            let response = client
                .post(format!("{}/subscriptions/{}/cancel", api_url.trim_end_matches('/'), subscription_id))
                .header("x-api-key", api_key)
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Creem cancel failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!("Creem cancel failed: {} - {}", status, body);
                return Err(BridgeError::ProviderError(format!("Creem cancel failed: {}", status)));
            }

            info!("Creem subscription {} cancelled via API", subscription_id);
            Ok(())
        }

        "lemonsqueezy" => {
            let api_key = config_str(config, "api_key", "LemonSqueezy")?;

            let payload = serde_json::json!({
                "data": {
                    "type": "subscriptions",
                    "id": subscription_id,
                    "attributes": {
                        "cancelled": true
                    }
                }
            });

            let response = client
                .patch(format!("https://api.lemonsqueezy.com/v1/subscriptions/{}", subscription_id))
                .bearer_auth(api_key)
                .header("Accept", "application/vnd.api+json")
                .header("Content-Type", "application/vnd.api+json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("LemonSqueezy cancel failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!("LemonSqueezy cancel failed: {} - {}", status, body);
                return Err(BridgeError::ProviderError(format!("LemonSqueezy cancel failed: {}", status)));
            }

            info!("LemonSqueezy subscription {} cancelled via API", subscription_id);
            Ok(())
        }

        "google_play" => {
            let service_account_path = config_str(config, "service_account_json", "Google Play")?;
            let package_name = config_str(config, "package_name", "Google Play")?;
            let token = purchase_token
                .ok_or_else(|| BridgeError::ValidationError("purchase_token required for Google Play cancellation".to_string()))?;

            let gp_client = crate::services::google_play::client::GooglePlayClient::new(service_account_path)
                .map_err(|e| BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e)))?;

            gp_client.cancel_subscription(package_name, subscription_id, token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play cancel failed: {}", e)))?;

            info!("Google Play subscription {} cancelled via API", subscription_id);
            Ok(())
        }

        _ => Err(BridgeError::ValidationError(format!("Cancel not supported for provider: {}", provider))),
    }
}

/// Resume subscription with the payment provider
pub async fn resume_subscription(
    provider: &str,
    subscription_id: &str,
    config: &Value,
) -> Result<(), BridgeError> {
    let client = reqwest::Client::new();

    match provider {
        "creem" => {
            let api_key = config_str(config, "api_key", "Creem")?;
            let api_url = config.get("api_url")
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.creem.com");

            let response = client
                .post(format!("{}/subscriptions/{}/resume", api_url.trim_end_matches('/'), subscription_id))
                .header("x-api-key", api_key)
                .header("Content-Type", "application/json")
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Creem resume failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!("Creem resume failed: {} - {}", status, body);
                return Err(BridgeError::ProviderError(format!("Creem resume failed: {}", status)));
            }

            info!("Creem subscription {} resumed via API", subscription_id);
            Ok(())
        }

        "lemonsqueezy" => {
            let api_key = config_str(config, "api_key", "LemonSqueezy")?;

            let payload = serde_json::json!({
                "data": {
                    "type": "subscriptions",
                    "id": subscription_id,
                    "attributes": {
                        "cancelled": false
                    }
                }
            });

            let response = client
                .patch(format!("https://api.lemonsqueezy.com/v1/subscriptions/{}", subscription_id))
                .bearer_auth(api_key)
                .header("Accept", "application/vnd.api+json")
                .header("Content-Type", "application/vnd.api+json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("LemonSqueezy resume failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!("LemonSqueezy resume failed: {} - {}", status, body);
                return Err(BridgeError::ProviderError(format!("LemonSqueezy resume failed: {}", status)));
            }

            info!("LemonSqueezy subscription {} resumed via API", subscription_id);
            Ok(())
        }

        _ => Err(BridgeError::ValidationError(format!("Resume not supported for provider: {}", provider))),
    }
}

/// Create billing portal URL via provider API
pub async fn create_billing_portal(
    provider: &str,
    provider_customer_id: &str,
    config: &Value,
) -> Result<String, BridgeError> {
    let client = reqwest::Client::new();

    match provider {
        "creem" => {
            let api_key = config_str(config, "api_key", "Creem")?;
            let api_url = config.get("api_url")
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.creem.com");

            let response = client
                .post(format!("{}/customers/billing", api_url.trim_end_matches('/')))
                .header("x-api-key", api_key)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "customer_id": provider_customer_id }))
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Creem billing portal failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!("Creem billing portal failed: {} - {}", status, body);
                return Err(BridgeError::ProviderError(format!("Creem billing portal failed: {}", status)));
            }

            let data: Value = response.json().await
                .map_err(|e| BridgeError::ProviderError(format!("Invalid billing portal response: {}", e)))?;

            data["url"].as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| BridgeError::ProviderError("Missing 'url' in billing portal response".to_string()))
        }

        "lemonsqueezy" => {
            let api_key = config_str(config, "api_key", "LemonSqueezy")?;

            let response = client
                .get(format!("https://api.lemonsqueezy.com/v1/customers/{}", provider_customer_id))
                .bearer_auth(api_key)
                .header("Accept", "application/vnd.api+json")
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("LemonSqueezy billing portal failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!("LemonSqueezy billing portal failed: {} - {}", status, body);
                return Err(BridgeError::ProviderError(format!("LemonSqueezy billing portal failed: {}", status)));
            }

            let data: Value = response.json().await
                .map_err(|e| BridgeError::ProviderError(format!("Invalid billing portal response: {}", e)))?;

            data["data"]["attributes"]["urls"]["customer_portal"].as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| BridgeError::ValidationError(
                    "Customer portal not available for this subscription".to_string(),
                ))
        }

        _ => Err(BridgeError::ValidationError(format!("Billing portal not supported for provider: {}", provider))),
    }
}

/// Fetch current subscription status from provider API (for reconciliation)
pub async fn fetch_subscription_status(
    provider: &str,
    subscription_id: &str,
    _purchase_token: Option<&str>,
    config: &Value,
) -> Result<(String, Option<chrono::DateTime<chrono::Utc>>), BridgeError> {
    let client = reqwest::Client::new();

    match provider {
        "creem" => {
            let api_key = config_str(config, "api_key", "Creem")?;
            let api_url = config.get("api_url")
                .and_then(|v| v.as_str())
                .unwrap_or("https://api.creem.com");

            let response = client
                .get(format!("{}/subscriptions/{}", api_url.trim_end_matches('/'), subscription_id))
                .header("x-api-key", api_key)
                .header("Content-Type", "application/json")
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Creem get subscription failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!("Creem get subscription failed: {} - {}", status, body);
                return Err(BridgeError::ProviderError(format!("Creem get subscription failed: {}", status)));
            }

            let data: Value = response.json().await
                .map_err(|e| BridgeError::ProviderError(format!("Invalid Creem response: {}", e)))?;

            let raw_status = data["status"].as_str().unwrap_or("unknown");
            let status = normalize_creem_status(raw_status);

            let period_end = data["renews_at"].as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            Ok((status, period_end))
        }

        "lemonsqueezy" => {
            let api_key = config_str(config, "api_key", "LemonSqueezy")?;

            let response = client
                .get(format!("https://api.lemonsqueezy.com/v1/subscriptions/{}", subscription_id))
                .bearer_auth(api_key)
                .header("Accept", "application/vnd.api+json")
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("LemonSqueezy get subscription failed: {}", e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!("LemonSqueezy get subscription failed: {} - {}", status, body);
                return Err(BridgeError::ProviderError(format!("LemonSqueezy get subscription failed: {}", status)));
            }

            let data: Value = response.json().await
                .map_err(|e| BridgeError::ProviderError(format!("Invalid LemonSqueezy response: {}", e)))?;

            let raw_status = data["data"]["attributes"]["status"].as_str().unwrap_or("unknown");
            let status = normalize_lemonsqueezy_status(raw_status);

            let period_end = data["data"]["attributes"]["renews_at"].as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            Ok((status, period_end))
        }

        "google_play" => {
            let service_account_path = config_str(config, "service_account_json", "Google Play")?;
            let package_name = config_str(config, "package_name", "Google Play")?;
            let token = _purchase_token
                .ok_or_else(|| BridgeError::ValidationError("purchase_token required for Google Play status fetch".to_string()))?;

            let gp_client = crate::services::google_play::client::GooglePlayClient::new(service_account_path)
                .map_err(|e| BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e)))?;

            let purchase = gp_client.get_subscription(package_name, subscription_id, token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play get subscription failed: {}", e)))?;

            let raw_status = purchase.subscription_state.as_deref().unwrap_or("unknown");
            let status = normalize_google_status(raw_status);

            let period_end = purchase.expiry_time.as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            Ok((status, period_end))
        }

        _ => Err(BridgeError::ValidationError(format!("Status fetch not supported for provider: {}", provider))),
    }
}

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

fn normalize_lemonsqueezy_status(raw: &str) -> String {
    match raw {
        "on_trial" => "trial".to_string(),
        "active" => "active".to_string(),
        "past_due" => "past_due".to_string(),
        "cancelled" => "cancelled".to_string(),
        "expired" => "expired".to_string(),
        "paused" => "paused".to_string(),
        "unpaid" => "past_due".to_string(),
        other => other.to_string(),
    }
}

fn normalize_google_status(raw: &str) -> String {
    match raw {
        "SUBSCRIPTION_STATE_ACTIVE" => "active".to_string(),
        "SUBSCRIPTION_STATE_CANCELED" => "cancelled".to_string(),
        "SUBSCRIPTION_STATE_IN_GRACE_PERIOD" => "past_due".to_string(),
        "SUBSCRIPTION_STATE_ON_HOLD" => "past_due".to_string(),
        "SUBSCRIPTION_STATE_PAUSED" => "paused".to_string(),
        "SUBSCRIPTION_STATE_PENDING" => "pending".to_string(),
        "SUBSCRIPTION_STATE_EXPIRED" => "expired".to_string(),
        _ => "active".to_string(), // Default to active for unknown to avoid false expiry
    }
}
