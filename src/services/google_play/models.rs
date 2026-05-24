use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct PubSubMessage {
    pub message: PubSubMessageInner,
    pub subscription: String,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct PubSubMessageInner {
    pub data: String, // Base64 encoded
    pub message_id: String,
    // other fields like publish_time, etc.
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeveloperNotification {
    pub version: String,
    #[serde(alias = "packageName")]
    pub package_name: String,
    #[serde(alias = "eventTimeMillis")]
    pub event_time_millis: String,
    #[serde(alias = "subscriptionNotification")]
    pub subscription_notification: Option<SubscriptionNotification>,
    #[serde(alias = "oneTimeProductNotification")]
    pub one_time_product_notification: Option<OneTimeProductNotification>,
    #[serde(alias = "testNotification")]
    pub test_notification: Option<TestNotification>,
    #[serde(alias = "voidedPurchaseNotification")]
    pub voided_purchase_notification: Option<VoidedPurchaseNotification>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionNotification {
    pub version: String,
    pub notification_type: i32,
    pub purchase_token: String,
    #[serde(alias = "subscriptionId")]
    pub subscription_id: String,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneTimeProductNotification {
    pub version: String,
    pub notification_type: i32,
    pub purchase_token: String,
    #[serde(alias = "productId", alias = "sku")]
    pub product_id: String,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestNotification {
    pub version: String,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoidedPurchaseNotification {
    pub purchase_token: String,
    pub order_id: String,
    pub product_type: i32, // 0 = OTP, 1 = subscription
    pub refund_type: i32,  // 0 = full refund, 1 = partial refund
}

// Structs for Google Play API responses
// Structs for Google Play API responses
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionPurchaseV2 {
    /// The kind of resource. Always androidpublisher#subscriptionPurchaseV2.
    pub kind: Option<String>,
    /// ISO 8601 format timestamp when the subscription started.
    pub start_time: Option<String>,
    /// ISO 8601 format timestamp when the subscription expires.
    pub expiry_time: Option<String>,
    /// Whether the subscription will auto-renew.
    pub auto_renewing: Option<bool>,
    /// The current state of the subscription.
    /// Values: SUBSCRIPTION_STATE_PENDING, SUBSCRIPTION_STATE_ACTIVE, SUBSCRIPTION_STATE_PAUSED,
    /// SUBSCRIPTION_STATE_IN_GRACE_PERIOD, SUBSCRIPTION_STATE_ON_HOLD, SUBSCRIPTION_STATE_CANCELED,
    /// SUBSCRIPTION_STATE_EXPIRED
    pub subscription_state: Option<String>,
    /// The order id of the latest order.
    pub latest_order_id: Option<String>,
    /// The purchase token of the old subscription if this subscription is one of the following:
    /// * Re-signup of a canceled but non-lapsed subscription
    /// * Upgrade/Downgrade from a previous subscription
    pub linked_purchase_token: Option<String>,
    /// The acknowledgment state of the subscription.
    /// Values: ACKNOWLEDGEMENT_STATE_PENDING, ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED
    pub acknowledgement_state: Option<String>,
    /// The line items of the subscription.
    pub line_items: Vec<SubscriptionLineItem>,
    /// User account identifiers for the subscription.
    pub external_account_identifiers: Option<ExternalAccountIdentifiers>,
    /// Context for out-of-app purchases (e.g. invalid re-signups).
    // pub subscribe_with_google_info: Option<SubscribeWithGoogleInfo>, // Omitted for now unless needed
    /// The cancellation reason, if applicable.
    /// Values: SUBSCRIPTION_CANCELED_USER_CANCELED, SUBSCRIPTION_CANCELED_SYSTEM_CANCELED,
    /// SUBSCRIPTION_CANCELED_REPLACED, SUBSCRIPTION_CANCELED_DEVELOPER_INITIATED
    pub canceled_state_context: Option<CanceledStateContext>,
    pub test_purchase: Option<serde_json::Value>, // Using Value for loosely defined object
    /// Details about a pending price change.
    pub price_change_summary: Option<PriceChangeSummary>,
    /// Context for out-of-app purchases (e.g., user resubscribes after expiry)
    /// Contains expired subscription identifiers for resubscription account linking
    pub out_of_app_purchase_context: Option<OutOfAppPurchaseContext>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PriceChangeSummary {
    pub new_price: Option<Money>,
    /// The current state of the price change.
    /// Values: PRICE_CHANGE_STATE_UNSPECIFIED, PRICE_CHANGE_STATE_OUTSTANDING, PRICE_CHANGE_STATE_ACCEPTED
    pub price_change_state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PriceChangeDetails {
    pub new_price: Option<Money>,
    pub price_change_mode: Option<String>,
    pub price_change_state: Option<String>,
    pub expected_new_price_charge_time: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub currency_code: Option<String>,
    pub units: Option<String>,
    pub nanos: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionLineItem {
    pub product_id: String,
    pub expiry_time: Option<String>,
    pub latest_successful_order_id: Option<String>,
    pub auto_renewing_plan: Option<AutoRenewingPlan>,
    pub offer_details: Option<OfferDetails>,
    pub offer_phase: Option<OfferPhase>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AutoRenewingPlan {
    pub auto_renew_enabled: Option<bool>,
    pub recurring_price: Option<Money>,
    pub price_change_details: Option<PriceChangeDetails>,
    // subscription_notes omitted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_auto_renewing_plan_price_change_details() {
        let value = serde_json::json!({
            "subscriptionState": "SUBSCRIPTION_STATE_ACTIVE",
            "lineItems": [{
                "productId": "hiha_monthly",
                "autoRenewingPlan": {
                    "autoRenewEnabled": true,
                    "priceChangeDetails": {
                        "newPrice": {
                            "currencyCode": "RON",
                            "units": "7",
                            "nanos": 490000000
                        },
                        "priceChangeMode": "PRICE_INCREASE",
                        "priceChangeState": "OUTSTANDING",
                        "expectedNewPriceChargeTime": "2026-05-22T16:58:21.621Z"
                    }
                }
            }]
        });

        let purchase: SubscriptionPurchaseV2 = serde_json::from_value(value).unwrap();
        let details = purchase.line_items[0]
            .auto_renewing_plan
            .as_ref()
            .and_then(|plan| plan.price_change_details.as_ref())
            .unwrap();

        assert_eq!(details.new_price.as_ref().and_then(|p| p.currency_code.as_deref()), Some("RON"));
        assert_eq!(details.price_change_mode.as_deref(), Some("PRICE_INCREASE"));
        assert_eq!(details.price_change_state.as_deref(), Some("OUTSTANDING"));
        assert_eq!(details.expected_new_price_charge_time.as_deref(), Some("2026-05-22T16:58:21.621Z"));
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OfferDetails {
    pub offer_id: Option<String>,
    pub base_plan_id: String,
    pub offer_tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OfferPhase {
    pub free_trial: Option<serde_json::Value>,
    pub base_price: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAccountIdentifiers {
    pub obfuscated_account_id: Option<String>,
    pub obfuscated_profile_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CanceledStateContext {
    pub user_initiated_cancellation: Option<UserInitiatedCancellation>,
    pub system_initiated_cancellation: Option<SystemInitiatedCancellation>,
    pub developer_initiated_cancellation: Option<DeveloperInitiatedCancellation>,
    pub replacement_cancellation: Option<ReplacementCancellation>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UserInitiatedCancellation {
    pub cancel_survey_result: Option<CancelSurveyResult>,
    pub cancel_time: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SystemInitiatedCancellation {
    pub cancel_time: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeveloperInitiatedCancellation {
    pub cancel_time: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementCancellation {
    pub cancel_time: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CancelSurveyResult {
    pub reason: Option<String>,
    pub reason_user_input: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProductPurchase {
    pub kind: String,
    pub purchase_state: i32,
    pub purchase_time_millis: String,
    pub order_id: Option<String>,
    /// User ID hash from Google Play. Must match hash of authenticated user ID for security.
    /// This validates the purchase belongs to the current user.
    pub obfuscated_account_id: Option<String>,
    /// Acknowledgment state of the product.
    /// 0 = Yet to be acknowledged, 1 = Acknowledged
    pub acknowledgement_state: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OutOfAppPurchaseContext {
    /// Expired external account identifiers for resubscription account linking
    pub expired_external_account_identifiers: Option<ExternalAccountIdentifiers>,
    /// The original purchase token from the expired subscription
    pub expired_purchase_token: Option<String>,
}
