use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    pub external_user_id: String,
    pub email: String,
    pub provider: String,
    pub product_id: String,
    pub product_type: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CheckoutResponse {
    pub checkout_id: String,
    #[serde(default)]
    pub provider: String,
    pub redirect_url: Option<String>,
    pub mobile_checkout_data: Option<serde_json::Value>,
}

pub(crate) struct CheckoutRedirectUrls {
    pub(crate) success_url: String,
    pub(crate) cancel_url: String,
}
