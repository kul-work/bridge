use crate::ports::WebhookProviderSnapshot;
use crate::error::BridgeError;
use crate::webhooks::provider_adapter::ProviderWebhookAdapter;

#[derive(Debug, Clone, Default)]
pub(crate) struct WebhookFields {
    pub(crate) subscription_id: Option<String>,
    pub(crate) purchase_token: Option<String>,
    pub(crate) amount_cents: Option<i64>,
    pub(crate) auto_renewing: Option<bool>,
    pub(crate) current_period_end: Option<String>,
    pub(crate) provider_transaction_id: Option<String>,
    pub(crate) provider_customer_id: Option<String>,
    pub(crate) product_id: Option<String>,
    pub(crate) cancel_reason: Option<String>,
    pub(crate) currency: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) google_subscription_state: Option<i32>,
    pub(crate) google_cancellation_context: Option<String>,
    pub(crate) google_cancellation_feedback: Option<String>,
    pub(crate) google_new_price_cents: Option<i64>,
    pub(crate) google_price_step_up_consent_deadline: Option<String>,
    pub(crate) google_pending_price_change_new_price_cents: Option<i64>,
    pub(crate) google_pending_price_change_currency: Option<String>,
    pub(crate) google_pending_price_change_mode: Option<String>,
    pub(crate) google_pending_price_change_state: Option<String>,
    pub(crate) google_pending_price_change_expected_at: Option<String>,
}

pub(super) fn extract_metadata_user_id(payload: &serde_json::Value) -> Option<String> {
    [
        "/metadata/user_id",
        "/object/metadata/user_id",
        "/object/checkout/metadata/user_id",
        "/event/data/metadata/external_user_id",
        "/event/data/metadata/user_id",
    ]
    .into_iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(|value| value.as_str()).map(|value| value.to_string()))
}

pub(super) fn extract_webhook_fields(webhook: &WebhookProviderSnapshot) -> Result<WebhookFields, BridgeError> {
    let adapter = ProviderWebhookAdapter::from_provider(&webhook.provider)?;
    Ok(adapter.extract_fields(&webhook.event_type, &webhook.payload))
}
