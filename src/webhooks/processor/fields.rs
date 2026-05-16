use crate::ports::WebhookProviderSnapshot;

use super::normalize::normalize_event_type_with_payload;

pub(crate) struct WebhookFields {
    pub(crate) subscription_id: Option<String>,
    pub(crate) purchase_token: Option<String>,
    pub(crate) amount_cents: Option<i32>,
    pub(crate) auto_renewing: Option<bool>,
    pub(crate) current_period_end: Option<String>,
    pub(crate) provider_transaction_id: Option<String>,
    pub(crate) provider_customer_id: Option<String>,
    pub(crate) product_id: Option<String>,
    pub(crate) cancel_reason: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) google_subscription_state: Option<i32>,
    pub(crate) google_cancellation_context: Option<String>,
    pub(crate) google_cancellation_feedback: Option<String>,
    pub(crate) google_new_price_cents: Option<i32>,
    pub(crate) google_price_step_up_consent_deadline: Option<String>,
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

pub(super) fn extract_webhook_fields(webhook: &WebhookProviderSnapshot) -> WebhookFields {
    let p = &webhook.payload;
    match webhook.provider.as_str() {
        "google_play" => WebhookFields {
            subscription_id: p.pointer("/subscriptionNotification/subscriptionId")
                .and_then(|v| v.as_str()).map(|s| s.to_string()),
            purchase_token: p.pointer("/subscriptionNotification/purchaseToken")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| p.pointer("/oneTimeProductNotification/purchaseToken")
                    .and_then(|v| v.as_str()).map(|s| s.to_string())),
            amount_cents: p.pointer("/oneTimeProductNotification/priceMicros")
                .and_then(|v| v.as_i64()).map(|m| (m / 10_000) as i32),
            auto_renewing: p.pointer("/subscriptionNotification/autoRenewing")
                .and_then(|v| v.as_bool()),
            current_period_end: p.pointer("/subscriptionNotification/expiryTimeMillis")
                .and_then(|v| v.as_i64())
                .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                .map(|dt| dt.to_rfc3339()),
            provider_transaction_id: p.pointer("/subscriptionNotification/orderId")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| p.pointer("/oneTimeProductNotification/purchaseToken")
                    .and_then(|v| v.as_str()).map(|s| s.to_string())),
            provider_customer_id: None,
            product_id: p.pointer("/subscriptionNotification/subscriptionId")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| p.pointer("/oneTimeProductNotification/sku")
                    .and_then(|v| v.as_str()).map(|s| s.to_string())),
            cancel_reason: p.pointer("/subscriptionNotification/cancelReason")
                .and_then(|v| v.as_i64()).map(|c| c.to_string()),
            status: None,
            google_subscription_state: None,
            google_cancellation_context: None,
            google_cancellation_feedback: None,
            google_new_price_cents: p.pointer("/subscriptionNotification/priceStepUpConsentDetails/priceMicros")
                .and_then(|v| v.as_i64()).map(|m| (m / 10_000) as i32),
            google_price_step_up_consent_deadline: p.pointer("/subscriptionNotification/priceStepUpConsentDetails/consentDeadlineTimeMillis")
                .and_then(|v| v.as_i64())
                .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                .map(|dt| dt.to_rfc3339()),
        },
        "creem" => {
            let obj = p.get("object").unwrap_or(&serde_json::Value::Null);
            let raw_event_type = p.get("eventType").and_then(|v| v.as_str()).unwrap_or("");

            // Extract top-level identifiers with fallbacks
            let object_id = obj.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
            let object_subscription_id = obj.get("subscription_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| obj.get("subscription")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()));
            let object_checkout_id = obj.get("checkout_id").and_then(|v| v.as_str()).map(|s| s.to_string());
            let object_order_id = obj.get("order_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| obj.get("order")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()));

            // Extract product_id with fallbacks (direct, nested.id, checkout product)
            let object_product_id = obj.get("product_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| obj.get("product")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()))
                .or_else(|| obj.get("checkout")
                    .and_then(|v| v.get("product"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()));

            let normalized_event_type = normalize_event_type_with_payload(
                "creem",
                raw_event_type,
                Some(p),
            );
            let subscription_obj = if raw_event_type == "checkout.completed"
                && normalized_event_type == "subscription.created"
            {
                obj.get("subscription").unwrap_or(&serde_json::Value::Null).clone()
            } else {
                obj.clone()
            };

            // Determine subscription_id based on event type
            let subscription_id = match normalized_event_type.as_str() {
                "purchase.one_time" | "purchase.one_time_refunded" => None,
                "payment.refunded" => object_subscription_id.clone()
                    .or_else(|| object_product_id.clone())
                    .or_else(|| object_id.clone()),
                "subscription.created" => {
                    if raw_event_type == "checkout.completed" {
                        subscription_obj.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
                            .or_else(|| object_subscription_id.clone())
                    } else {
                        object_subscription_id.clone().or_else(|| object_id.clone())
                    }
                }
                _ => object_subscription_id.clone().or_else(|| object_id.clone()),
            };

            // Extract status from subscription object (for checkout.completed with recurring, nested in subscription)
            let status = subscription_obj.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());

            // Extract current_period_end with fallback to renews_at
            let current_period_end = subscription_obj.get("current_period_end_date")
                .and_then(|v| v.as_str())
                .or_else(|| subscription_obj.get("renews_at").and_then(|v| v.as_str()))
                .map(|s| s.to_string());

            // Extract amount with multiple fallbacks (last_transaction.amount, order.amount, product.price, amount)
            let amount_cents = obj.get("last_transaction")
                .and_then(|v| v.get("amount"))
                .and_then(|v| v.as_i64())
                .or_else(|| obj.get("order")
                    .and_then(|v| v.get("amount"))
                    .and_then(|v| v.as_i64()))
                .or_else(|| obj.get("product")
                    .and_then(|v| v.get("price"))
                    .and_then(|v| v.as_i64()))
                .or_else(|| obj.get("amount").and_then(|v| v.as_i64()))
                .map(|a| a as i32);

            // Extract purchase_token (checkout_id for OTP, order_id for refunds)
            let purchase_token = match normalized_event_type.as_str() {
                "purchase.one_time" | "purchase.one_time_refunded" => object_checkout_id
                    .or_else(|| object_order_id.clone())
                    .or_else(|| object_id.clone()),
                "payment.refunded" => object_order_id
                    .or(object_checkout_id)
                    .or_else(|| object_subscription_id.clone())
                    .or_else(|| object_id.clone()),
                "payment.partially_refunded" => object_order_id
                    .or(object_checkout_id)
                    .or_else(|| object_id.clone()),
                _ => None,
            };

            // Extract provider_transaction_id (last_transaction_id, fallback to object.id)
            let provider_transaction_id = obj.get("last_transaction_id")
                .and_then(|v| v.as_str()).map(|s| s.to_string())
                .or_else(|| object_id.clone());

            WebhookFields {
                subscription_id,
                purchase_token,
                amount_cents,
                auto_renewing: obj.get("auto_renewing").and_then(|v| v.as_bool()),
                current_period_end,
                provider_transaction_id,
                provider_customer_id: obj.get("customer")
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                product_id: object_product_id,
                cancel_reason: None,
                status,
                google_subscription_state: None,
                google_cancellation_context: None,
                google_cancellation_feedback: None,
                google_new_price_cents: None,
                google_price_step_up_consent_deadline: None,
            }
        }
        _ => WebhookFields {
            subscription_id: None,
            purchase_token: None,
            amount_cents: None,
            auto_renewing: None,
            current_period_end: None,
            provider_transaction_id: None,
            provider_customer_id: None,
            product_id: None,
            cancel_reason: None,
            status: None,
            google_subscription_state: None,
            google_cancellation_context: None,
            google_cancellation_feedback: None,
            google_new_price_cents: None,
            google_price_step_up_consent_deadline: None,
        },
    }
}
