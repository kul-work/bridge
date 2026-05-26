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
}
