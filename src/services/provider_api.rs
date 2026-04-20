use crate::error::BridgeError;
use crate::services::creem::client::CreemClient;
use serde_json::Value;
use tracing::info;

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
    match provider {
        "creem" => {
            let creem_client = CreemClient::from_json(config)?;
            creem_client.cancel_subscription(subscription_id, mode).await?;
            info!("Creem subscription {} cancelled via API", subscription_id);
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
    match provider {
        "creem" => {
            let creem_client = CreemClient::from_json(config)?;
            creem_client.resume_subscription(subscription_id).await?;
            info!("Creem subscription {} resumed via API", subscription_id);
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
    match provider {
        "creem" => {
            let creem_client = CreemClient::from_json(config)?;
            creem_client.create_billing_portal(provider_customer_id).await
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
    match provider {
        "creem" => {
            let creem_client = CreemClient::from_json(config)?;
            creem_client.fetch_subscription_status(subscription_id).await
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
