use serde::{Deserialize, Serialize};

/// Creem create checkout request
#[derive(Debug, Clone, Serialize)]
pub struct CreateCheckoutRequest {
    pub product_id: String,
    pub customer: CustomerData,
    pub metadata: serde_json::Value,
    pub success_url: String,
    pub cancel_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CustomerData {
    pub email: String,
}

/// Creem create checkout response
#[derive(Debug, Clone, Deserialize)]
pub struct CreateCheckoutResponse {
    pub id: Option<String>,
    pub checkout_url: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub data: Option<CreateCheckoutResponseData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateCheckoutResponseData {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub attributes: Option<CreateCheckoutResponseAttributes>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateCheckoutResponseAttributes {
    #[serde(default)]
    pub url: Option<String>,
}

impl CreateCheckoutResponse {
    /// Extract checkout URL from response (handles multiple possible field names)
    pub fn get_checkout_url(&self) -> Option<&str> {
        self.checkout_url
            .as_deref()
            .or(self.url.as_deref())
            .or_else(|| self.data.as_ref().and_then(|data| data.url.as_deref()))
            .or_else(|| {
                self.data
                    .as_ref()
                    .and_then(|data| data.attributes.as_ref())
                    .and_then(|attributes| attributes.url.as_deref())
            })
    }

    /// Extract session ID from response
    pub fn get_session_id(&self) -> Option<&str> {
        self.id
            .as_deref()
            .or_else(|| self.data.as_ref().and_then(|data| data.id.as_deref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_checkout_response_prefers_top_level_fields() {
        let json = serde_json::json!({
            "id": "co_top",
            "checkout_url": "https://checkout.creem.io/top",
            "data": {
                "id": "co_nested",
                "url": "https://checkout.creem.io/nested"
            }
        });

        let parsed: CreateCheckoutResponse = serde_json::from_value(json).expect("response should parse");
        assert_eq!(parsed.get_session_id(), Some("co_top"));
        assert_eq!(parsed.get_checkout_url(), Some("https://checkout.creem.io/top"));
    }

    #[test]
    fn create_checkout_response_supports_data_url_and_id() {
        let json = serde_json::json!({
            "data": {
                "id": "co_nested",
                "url": "https://checkout.creem.io/nested"
            }
        });

        let parsed: CreateCheckoutResponse = serde_json::from_value(json).expect("response should parse");
        assert_eq!(parsed.get_session_id(), Some("co_nested"));
        assert_eq!(parsed.get_checkout_url(), Some("https://checkout.creem.io/nested"));
    }

    #[test]
    fn create_checkout_response_supports_data_attributes_url() {
        let json = serde_json::json!({
            "data": {
                "id": "co_attr",
                "attributes": {
                    "url": "https://checkout.creem.io/attributes"
                }
            }
        });

        let parsed: CreateCheckoutResponse = serde_json::from_value(json).expect("response should parse");
        assert_eq!(parsed.get_session_id(), Some("co_attr"));
        assert_eq!(parsed.get_checkout_url(), Some("https://checkout.creem.io/attributes"));
    }
}

/// Creem cancel/resume request
#[derive(Debug, Clone, Serialize)]
pub struct ModifySubscriptionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Creem billing portal response
#[derive(Debug, Clone, Deserialize)]
pub struct BillingPortalResponse {
    pub url: String,
}

/// Creem subscription status response
#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionStatusResponse {
    pub status: String,
    pub renews_at: Option<String>,
}
