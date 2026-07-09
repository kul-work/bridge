use crate::error::BridgeError;
use crate::services::creem::client::CreemClient;
use crate::services::creem::config::CreemConfig;
use crate::services::google_play::status::{
    subscription_state_to_canonical_status, GoogleSubscriptionStateStatus,
};
use serde_json::Value;
use tracing::{info, warn};

/// Extract a config string field or return ConfigError
fn config_str<'a>(config: &'a Value, key: &str, provider: &str) -> Result<&'a str, BridgeError> {
    config.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError(format!("Missing {} {} config", provider, key)))
}

fn configured_provider_api_url_is_localhost(provider: &str, config: &Value) -> bool {
    match provider {
        "creem" => CreemConfig::from_json(config)
            .map(|creem_config| crate::config::is_localhost_url(creem_config.base_url()))
            .unwrap_or(false),
        _ => false,
    }
}

fn should_skip_non_localhost_provider_call(provider: &str, operation: &str, config: &Value) -> bool {
    if !crate::config::mock_external_apis_enabled() {
        return false;
    }

    if configured_provider_api_url_is_localhost(provider, config) {
        return false;
    }

    info!(
        provider,
        operation,
        "MOCK_EXTERNAL_APIS: Skipping non-localhost provider API call"
    );
    true
}

/// Cancel subscription with the payment provider
pub async fn cancel_subscription(
    provider: &str,
    subscription_id: &str,
    purchase_token: Option<&str>,
    mode: Option<&str>,
    on_execute: Option<&str>,
    config: &Value,
) -> Result<(), BridgeError> {
    match provider {
        "creem" => {
            if should_skip_non_localhost_provider_call(provider, "cancel_subscription", config) {
                return Ok(());
            }

            let creem_client = CreemClient::from_json(config)?;
            creem_client.cancel_subscription(subscription_id, mode, on_execute).await?;
            info!("Creem subscription {} cancelled via API", subscription_id);
            Ok(())
        }

        "google_play" => {
            if should_skip_non_localhost_provider_call(provider, "cancel_subscription", config) {
                return Ok(());
            }

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
            if should_skip_non_localhost_provider_call(provider, "resume_subscription", config) {
                return Ok(());
            }

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
            if should_skip_non_localhost_provider_call(provider, "create_billing_portal", config) {
                return Ok("http://localhost/mock-billing-portal".to_string());
            }

            let creem_client = CreemClient::from_json(config)?;
            creem_client.create_billing_portal(provider_customer_id).await
        }

        _ => Err(BridgeError::ValidationError(format!("Billing portal not supported for provider: {}", provider))),
    }
}

/// Acknowledge a subscription purchase with the payment provider.
pub async fn acknowledge_subscription(
    provider: &str,
    subscription_id: &str,
    purchase_token: &str,
    config: &Value,
) -> Result<(), BridgeError> {
    match provider {
        "google_play" => {
            if should_skip_non_localhost_provider_call(provider, "acknowledge_subscription", config) {
                return Ok(());
            }

            let service_account_path = config_str(config, "service_account_json", "Google Play")?;
            let package_name = config_str(config, "package_name", "Google Play")?;

            let gp_client = crate::services::google_play::client::GooglePlayClient::new(service_account_path)
                .map_err(|e| BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e)))?;

            gp_client.acknowledge_subscription(package_name, subscription_id, purchase_token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play acknowledgement failed: {}", e)))?;

            info!("Google Play subscription {} acknowledged via API", subscription_id);
            Ok(())
        }

        _ => Err(BridgeError::ValidationError(format!("Acknowledgement not supported for provider: {}", provider))),
    }
}

/// Acknowledge a one-time product purchase with the payment provider.
pub async fn acknowledge_product(
    provider: &str,
    product_id: &str,
    purchase_token: &str,
    config: &Value,
) -> Result<(), BridgeError> {
    match provider {
        "google_play" => {
            if should_skip_non_localhost_provider_call(provider, "acknowledge_product", config) {
                return Ok(());
            }

            let service_account_path = config_str(config, "service_account_json", "Google Play")?;
            let package_name = config_str(config, "package_name", "Google Play")?;

            let gp_client = crate::services::google_play::client::GooglePlayClient::new(service_account_path)
                .map_err(|e| BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e)))?;

            gp_client.acknowledge(package_name, product_id, purchase_token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play product acknowledgement failed: {}", e)))?;

            info!("Google Play product {} acknowledged via API", product_id);
            Ok(())
        }

        _ => Err(BridgeError::ValidationError(format!("Acknowledgement not supported for provider: {}", provider))),
    }
}

/// Fetch current subscription status from provider API (for reconciliation)
pub async fn fetch_subscription_status(
    provider: &str,
    subscription_id: &str,
    _purchase_token: Option<&str>,
    config: &Value,
) -> Result<(Option<String>, Option<chrono::DateTime<chrono::Utc>>), BridgeError> {
    match provider {
        "creem" => {
            if should_skip_non_localhost_provider_call(provider, "fetch_subscription_status", config) {
                return Ok((None, None));
            }

            let creem_client = CreemClient::from_json(config)?;
            creem_client.fetch_subscription_status(subscription_id).await
        }

        "google_play" => {
            if should_skip_non_localhost_provider_call(provider, "fetch_subscription_status", config) {
                return Ok((None, None));
            }

            let service_account_path = config_str(config, "service_account_json", "Google Play")?;
            let package_name = config_str(config, "package_name", "Google Play")?;
            let token = _purchase_token
                .ok_or_else(|| BridgeError::ValidationError("purchase_token required for Google Play status fetch".to_string()))?;

            let gp_client = crate::services::google_play::client::GooglePlayClient::new(service_account_path)
                .map_err(|e| BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e)))?;

            let purchase = gp_client.get_subscription(package_name, subscription_id, token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play get subscription failed: {}", e)))?;

            let raw_status = purchase.subscription_state.as_deref();
            let status = normalize_google_status(raw_status);

            let period_end = purchase.expiry_time.as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            Ok((status, period_end))
        }

        _ => Err(BridgeError::ValidationError(format!("Status fetch not supported for provider: {}", provider))),
    }
}

fn normalize_google_status(raw: Option<&str>) -> Option<String> {
    match subscription_state_to_canonical_status(raw) {
        GoogleSubscriptionStateStatus::Known(status) => Some(status.to_string()),
        GoogleSubscriptionStateStatus::Unknown(other) => {
            warn!(raw_status = other, "unknown Google Play subscription status ignored");
            None
        }
        GoogleSubscriptionStateStatus::Missing => {
            warn!("Google Play subscription status missing");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_unknown_subscription_status_is_not_treated_as_active() {
        assert_eq!(normalize_google_status(Some("SUBSCRIPTION_STATE_ACTIVE")), Some("active".to_string()));
        assert_eq!(normalize_google_status(Some("SUBSCRIPTION_STATE_FUTURE")), None);
        assert_eq!(normalize_google_status(None), None);
    }
}
