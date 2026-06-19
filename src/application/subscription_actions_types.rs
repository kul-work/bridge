use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct SubscriptionActionQuery {
    pub external_user_id: String,
    pub provider: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelSubscriptionRequest {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub purchase_token: Option<String>,
    #[serde(default)]
    pub on_execute: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionActionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ResumeSubscriptionResponse {
    pub status: String,
    pub subscription_id: String,
}

#[derive(Debug, Serialize)]
pub struct CancelSubscriptionResponse {
    pub status: String,
    pub mode: String,
    pub subscription_id: String,
}

#[derive(Debug, Serialize)]
pub struct BillingPortalResponse {
    pub url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceStepUpRequest {
    pub external_user_id: String,
}

#[derive(Debug, Serialize)]
pub struct PriceStepUpAcceptResponse {
    pub accepted: bool,
    pub new_price_cents: i32,
}

#[derive(Debug, Serialize)]
pub struct PriceStepUpDeclineResponse {
    pub declined: bool,
    pub cancellation_effective_at: String,
}
