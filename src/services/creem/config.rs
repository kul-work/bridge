use crate::error::BridgeError;
use serde_json::Value;

/// Creem configuration from provider_configs.config JSON
#[derive(Debug, Clone)]
pub struct CreemConfig {
    pub api_key: String,
    pub api_url: String,
    pub product_id: Option<String>,
    pub offer_id: Option<String>,
    pub otp_id: Option<String>,
    pub connect_timeout_secs: u64,
    pub request_timeout_secs: u64,
}

impl CreemConfig {
    /// Parse Creem configuration from provider_configs.config JSON
    pub fn from_json(config: &Value) -> Result<Self, BridgeError> {
        let api_key = config
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BridgeError::ConfigError("Missing Creem api_key".to_string()))?
            .to_string();

        let api_url = config
            .get("api_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.creem.com")
            .to_string();
        validate_creem_api_url(&api_url)?;

        let product_id = config
            .get("product_id")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string());

        let offer_id = config
            .get("offer_id")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string());

        let otp_id = config
            .get("otp_id")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string());

        let connect_timeout_secs = config
            .get("connect_timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(5);
        if connect_timeout_secs == 0 {
            return Err(BridgeError::ConfigError(
                "Invalid Creem connect_timeout_secs: must be greater than 0".to_string(),
            ));
        }

        let request_timeout_secs = config
            .get("request_timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(25);
        if request_timeout_secs == 0 {
            return Err(BridgeError::ConfigError(
                "Invalid Creem request_timeout_secs: must be greater than 0".to_string(),
            ));
        }

        Ok(Self {
            api_key,
            api_url,
            product_id,
            offer_id,
            otp_id,
            connect_timeout_secs,
            request_timeout_secs,
        })
    }

    /// Get API URL without trailing slash
    pub fn base_url(&self) -> &str {
        self.api_url.trim_end_matches('/')
    }
}

fn validate_creem_api_url(url_str: &str) -> Result<(), BridgeError> {
    let parsed = url::Url::parse(url_str).map_err(|e| {
        BridgeError::ConfigError(format!("Invalid Creem api_url '{}': {}", url_str, e))
    })?;

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(BridgeError::ConfigError(format!(
            "Invalid Creem api_url '{}': userinfo is not allowed", url_str
        )));
    }

    let host = parsed.host_str().unwrap_or("");
    let is_localhost = host == "localhost" || host == "127.0.0.1";
    let is_creem = host == "api.creem.com"
        || host == "api.creem.io"
        || host.ends_with(".creem.com")
        || host.ends_with(".creem.io");

    if !is_localhost && !is_creem {
        return Err(BridgeError::ConfigError(format!(
            "Invalid Creem api_url '{}': host must be a creem.com or creem.io domain", url_str
        )));
    }

    if is_localhost {
        let localhost_allowed = cfg!(test) || crate::config::mock_external_apis_enabled();
        if !localhost_allowed {
            return Err(BridgeError::ConfigError(format!(
                "Invalid Creem api_url '{}': localhost is only allowed in mock/test mode", url_str
            )));
        }
    } else if parsed.scheme() != "https" {
        return Err(BridgeError::ConfigError(format!(
            "Invalid Creem api_url '{}': must use HTTPS", url_str
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_optional_offer_and_otp_ids() {
        let config = serde_json::json!({
            "api_key": "sk_test_123"
        });

        let parsed = CreemConfig::from_json(&config).expect("config should parse");
        assert_eq!(parsed.product_id, None);
        assert_eq!(parsed.offer_id, None);
        assert_eq!(parsed.otp_id, None);
    }

    #[test]
    fn supports_product_offer_and_otp_ids() {
        let config = serde_json::json!({
            "api_key": "sk_test_123",
            "product_id": "standard_monthly",
            "offer_id": "offer_monthly",
            "otp_id": "otp_lifetime"
        });

        let parsed = CreemConfig::from_json(&config).expect("config should parse");
        assert_eq!(parsed.product_id.as_deref(), Some("standard_monthly"));
        assert_eq!(parsed.offer_id.as_deref(), Some("offer_monthly"));
        assert_eq!(parsed.otp_id.as_deref(), Some("otp_lifetime"));
    }

    #[test]
    fn rejects_http_api_url() {
        let config = serde_json::json!({
            "api_key": "sk_test_123",
            "api_url": "http://api.creem.com"
        });
        assert!(CreemConfig::from_json(&config).is_err());
    }

    #[test]
    fn allows_http_localhost_in_test_builds() {
        let config = serde_json::json!({
            "api_key": "sk_test_123",
            "api_url": "http://127.0.0.1:8080"
        });
        assert!(CreemConfig::from_json(&config).is_ok());
    }

    #[test]
    fn rejects_userinfo_bypass() {
        let config = serde_json::json!({
            "api_key": "sk_test_123",
            "api_url": "https://api.creem.com:443@evil.example.com/v1"
        });
        assert!(CreemConfig::from_json(&config).is_err());
    }

    #[test]
    fn rejects_non_creem_host() {
        let config = serde_json::json!({
            "api_key": "sk_test_123",
            "api_url": "https://evil.example.com"
        });
        assert!(CreemConfig::from_json(&config).is_err());
    }

    #[test]
    fn accepts_subdomain_of_creem() {
        let config = serde_json::json!({
            "api_key": "sk_test_123",
            "api_url": "https://eu.api.creem.com"
        });
        assert!(CreemConfig::from_json(&config).is_ok());
    }

    #[test]
    fn accepts_default_api_url() {
        let config = serde_json::json!({
            "api_key": "sk_test_123"
        });
        assert!(CreemConfig::from_json(&config).is_ok());
    }
}
