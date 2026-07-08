use axum::http::HeaderMap;
use base64::Engine;
use tracing::{error, info};
use crate::error::BridgeError;
use crate::utils::diagnostic_hash;
use crate::webhooks::processor::WebhookFields;


#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct NormalizedProviderEvent {
    pub(crate) provider: String,
    pub(crate) provider_event_id: String,
    pub(crate) raw_event_type: String,
    pub(crate) canonical_event_type: Option<String>,
    pub(crate) occurred_at_ms: Option<i64>,
    pub(crate) subscription_id: Option<String>,
    pub(crate) purchase_token: Option<String>,
    pub(crate) product_id: Option<String>,
    pub(crate) provider_transaction_id: Option<String>,
    pub(crate) payload: serde_json::Value,
}

pub(crate) fn normalize_google_play_event(
    payload: serde_json::Value,
    pubsub_message_id: Option<String>,
) -> Result<NormalizedProviderEvent, BridgeError> {
    let provider_event_id = pubsub_message_id
        .or_else(|| payload["eventId"].as_str().map(|value| value.to_string()))
        .ok_or_else(|| BridgeError::WebhookError("Missing provider event ID".to_string()))?;
    let raw_event_type = google_play_event_type(&payload);
    let subscription_id = payload["subscriptionNotification"]["subscriptionId"]
        .as_str()
        .map(|value| value.to_string());
    let purchase_token = payload["subscriptionNotification"]["purchaseToken"]
        .as_str()
        .or_else(|| payload["oneTimeProductNotification"]["purchaseToken"].as_str())
        .or_else(|| payload["voidedPurchaseNotification"]["purchaseToken"].as_str())
        .map(|value| value.to_string());
    let occurred_at_ms = payload["eventTimeMillis"]
        .as_str()
        .and_then(|value| value.parse::<i64>().ok())
        .or_else(|| payload["eventTimeMillis"].as_i64());

    Ok(NormalizedProviderEvent {
        provider: "google_play".to_string(),
        provider_event_id,
        raw_event_type,
        canonical_event_type: None,
        occurred_at_ms,
        subscription_id,
        purchase_token,
        product_id: None,
        provider_transaction_id: None,
        payload,
    })
}

pub(crate) fn normalize_creem_event(
    payload: serde_json::Value,
) -> Result<NormalizedProviderEvent, BridgeError> {
    let provider_event_id = payload["id"]
        .as_str()
        .map(|value| value.to_string())
        .ok_or_else(|| BridgeError::WebhookError("Missing provider event ID".to_string()))?;
    let raw_event_type = payload["eventType"].as_str().unwrap_or("unknown").to_string();
    let subscription_id = payload["object"]["subscription"]["id"]
        .as_str()
        .or_else(|| payload["object"]["subscription_id"].as_str())
        .or_else(|| payload["object"]["id"].as_str())
        .map(|value| value.to_string());
    let purchase_token = if raw_event_type.starts_with("subscription.") {
        payload["object"]["checkout_id"]
            .as_str()
            .or_else(|| payload["object"]["order_id"].as_str())
            .map(|value| value.to_string())
    } else {
        payload["object"]["checkout_id"]
            .as_str()
            .or_else(|| payload["object"]["order_id"].as_str())
            .or_else(|| payload["object"]["id"].as_str())
            .map(|value| value.to_string())
    };
    let occurred_at_ms = payload["createdAt"].as_str().and_then(|value| {
        chrono::DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|date| date.timestamp_millis())
    });

    Ok(NormalizedProviderEvent {
        provider: "creem".to_string(),
        provider_event_id,
        raw_event_type,
        canonical_event_type: None,
        occurred_at_ms,
        subscription_id,
        purchase_token,
        product_id: None,
        provider_transaction_id: None,
        payload,
    })
}

fn google_play_event_type(payload: &serde_json::Value) -> String {
    if let Some(notification_type) = payload["subscriptionNotification"]["notificationType"]
        .as_i64()
        .or_else(|| {
            payload["subscriptionNotification"]["notificationType"]
                .as_str()
                .and_then(|value| value.parse::<i64>().ok())
        })
    {
        return match notification_type {
            1 => "SUBSCRIPTION_RESTORED",
            2 => "SUBSCRIPTION_RENEWED",
            3 => "SUBSCRIPTION_CANCELED",
            4 => "SUBSCRIPTION_PURCHASED",
            5 => "SUBSCRIPTION_ON_HOLD",
            6 => "SUBSCRIPTION_IN_GRACE_PERIOD",
            7 => "SUBSCRIPTION_RESTARTED",
            8 => "SUBSCRIPTION_PRICE_CHANGE_CONFIRMED",
            9 => "SUBSCRIPTION_DEFERRED",
            10 => "SUBSCRIPTION_PAUSED",
            11 => "SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED",
            12 => "SUBSCRIPTION_REVOKED",
            13 => "SUBSCRIPTION_EXPIRED",
            17 => "SUBSCRIPTION_ITEMS_CHANGED",
            18 => "SUBSCRIPTION_CANCELLATION_SCHEDULED",
            19 => "SUBSCRIPTION_PRICE_CHANGE_UPDATED",
            20 => "SUBSCRIPTION_PENDING_PURCHASE_CANCELED",
            21 => "SUBSCRIPTION_RENEWAL_PENDING",
            22 => "SUBSCRIPTION_PRICE_STEP_UP_CONSENT_UPDATED",
            _ => "SUBSCRIPTION_UNKNOWN",
        }
        .to_string();
    }

    if let Some(notification_type) = payload["oneTimeProductNotification"]["notificationType"]
        .as_i64()
        .or_else(|| {
            payload["oneTimeProductNotification"]["notificationType"]
                .as_str()
                .and_then(|value| value.parse::<i64>().ok())
        })
    {
        return match notification_type {
            1 => "ONE_TIME_PRODUCT_PURCHASED",
            2 => "ONE_TIME_PRODUCT_REFUNDED",
            14 => "ONE_TIME_PRODUCT_CANCELED",
            _ => "ONE_TIME_PRODUCT_UNKNOWN",
        }
        .to_string();
    }

    if payload.get("voidedPurchaseNotification").is_some() {
        return "VOIDED_PURCHASE".to_string();
    }

    "unknown".to_string()
}

#[derive(Debug)]
pub(crate) enum ProviderWebhookAdapter {
    GooglePlay,
    Creem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NormalizedProviderStatus {
    Known(String),
    Unknown(String),
    Missing,
}

impl NormalizedProviderStatus {
    pub(crate) fn known(self) -> Option<String> {
        match self {
            Self::Known(status) => Some(status),
            Self::Unknown(_) | Self::Missing => None,
        }
    }
}

impl ProviderWebhookAdapter {
    pub(crate) fn from_provider(provider: &str) -> Result<Self, BridgeError> {
        match provider {
            "google_play" => Ok(Self::GooglePlay),
            "creem" => Ok(Self::Creem),
            _ => Err(BridgeError::ValidationError(format!("Unknown provider: {}", provider))),
        }
    }

    pub(crate) fn decode_and_normalize(
        &self,
        payload: serde_json::Value,
        headers: &HeaderMap,
    ) -> Result<Option<NormalizedProviderEvent>, BridgeError> {
        match self {
            Self::GooglePlay => {
                let (mut decoded_payload, pubsub_message_id) = decode_google_play_payload(&payload, headers)?;
                tracing::debug!(
                    target: "BPT-RAW",
                    "Webhook Incoming Payload [google_play]: {}",
                    sanitize_google_play_payload_for_log(&decoded_payload)
                );
                if decoded_payload.get("testNotification").is_some() {
                    info!(
                        message_id = pubsub_message_id.as_deref().unwrap_or("unknown"),
                        "Google Play test notification received; no-op"
                    );
                    return Ok(None);
                }
                if crate::config::mock_external_apis_enabled() {
                    if let Some(price_str) = headers.get("X-Test-Price-Cents").and_then(|h| h.to_str().ok()) {
                        if let Ok(cents) = price_str.parse::<i64>() {
                            decoded_payload["_test_price_cents"] = serde_json::Value::Number(cents.into());
                        }
                    }
                }
                let mut event = normalize_google_play_event(decoded_payload, pubsub_message_id)?;
                event.canonical_event_type = Some(self.canonical_event_type(&event.raw_event_type, &event.payload));
                Ok(Some(event))
            }
            Self::Creem => {
                let mut event = normalize_creem_event(payload)?;
                event.canonical_event_type = Some(self.canonical_event_type(&event.raw_event_type, &event.payload));
                Ok(Some(event))
            }
        }
    }

    pub(crate) fn canonical_event_type(
        &self,
        raw_event_type: &str,
        payload: &serde_json::Value,
    ) -> String {
        match self {
            Self::GooglePlay => {
                if raw_event_type == "VOIDED_PURCHASE"
                    && payload
                        .pointer("/voidedPurchaseNotification/productType")
                        .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok())))
                        == Some(2)
                {
                    return "purchase.one_time_refunded".to_string();
                }

                match raw_event_type {
                    "SUBSCRIPTION_PURCHASED" => "subscription.activated".to_string(),
                    "SUBSCRIPTION_RENEWED" => "subscription.renewed".to_string(),
                    "SUBSCRIPTION_CANCELED" => "subscription.cancelled".to_string(),
                    "SUBSCRIPTION_RESTORED" => "subscription.recovered".to_string(),
                    "SUBSCRIPTION_EXPIRED" => "subscription.expired".to_string(),
                    "SUBSCRIPTION_ON_HOLD" => "subscription.on_hold".to_string(),
                    "SUBSCRIPTION_IN_GRACE_PERIOD" => "subscription.grace_period".to_string(),
                    "SUBSCRIPTION_RESTARTED" => "subscription.resumed".to_string(),
                    "SUBSCRIPTION_PAUSED" => "subscription.paused".to_string(),
                    "SUBSCRIPTION_REVOKED" => "subscription.revoked".to_string(),
                    "SUBSCRIPTION_DEFERRED" => "subscription.deferred".to_string(),
                    "SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED" => "subscription.pause_scheduled".to_string(),
                    "SUBSCRIPTION_RENEWAL_PENDING" => "subscription.pending".to_string(),
                    "SUBSCRIPTION_ITEMS_CHANGED" => "subscription.items_changed".to_string(),
                    "SUBSCRIPTION_CANCELLATION_SCHEDULED" => "subscription.cancellation_scheduled".to_string(),
                    "SUBSCRIPTION_PRICE_CHANGE_CONFIRMED" => "subscription.price_changed".to_string(),
                    "SUBSCRIPTION_PRICE_CHANGE_UPDATED" => "subscription.price_change_updated".to_string(),
                    "SUBSCRIPTION_PRICE_STEP_UP_CONSENT_UPDATED" => "subscription.price_step_up".to_string(),
                    "SUBSCRIPTION_PENDING_PURCHASE_CANCELED" => "subscription.pending_purchase_cancelled".to_string(),
                    "ONE_TIME_PRODUCT_PURCHASED" => "purchase.one_time".to_string(),
                    "ONE_TIME_PRODUCT_REFUNDED" => "purchase.one_time_refunded".to_string(),
                    "ONE_TIME_PRODUCT_CANCELED" => "purchase.one_time_cancelled".to_string(),
                    "VOIDED_PURCHASE" => "payment.refunded".to_string(),
                    _ => format!("google_play.{}", raw_event_type),
                }
            }
            Self::Creem => {
                if raw_event_type == "refund.created" {
                    let object = payload.get("object").unwrap_or(&serde_json::Value::Null);
                    let billing_type = object.get("billing_type")
                        .and_then(|v| v.as_str())
                        .or_else(|| object.get("product")
                            .and_then(|v| v.get("billing_type"))
                            .and_then(|v| v.as_str()))
                        .or_else(|| object.get("order")
                            .and_then(|v| v.get("type"))
                            .and_then(|v| v.as_str()))
                        .or_else(|| object.get("checkout")
                            .and_then(|v| v.get("billing_type"))
                            .and_then(|v| v.as_str()));

                    if matches!(billing_type, Some("one_time") | Some("one-time") | Some("otp") | Some("lifetime")) {
                        return "purchase.one_time_refunded".to_string();
                    }
                }

                if raw_event_type == "checkout.completed" {
                    let object = payload.get("object").unwrap_or(&serde_json::Value::Null);
                    let billing_type = object.get("billing_type")
                        .and_then(|v| v.as_str())
                        .or_else(|| object.get("product")
                            .and_then(|v| v.get("billing_type"))
                            .and_then(|v| v.as_str()))
                        .or_else(|| object.get("order")
                            .and_then(|v| v.get("type"))
                            .and_then(|v| v.as_str()))
                        .or_else(|| object.get("subscription")
                            .and_then(|v| v.get("product"))
                            .and_then(|v| v.get("billing_type"))
                            .and_then(|v| v.as_str()));

                    if matches!(billing_type, Some("recurring") | Some("monthly")) {
                        return "subscription.created".to_string();
                    }

                    if matches!(billing_type, Some("one_time") | Some("one-time") | Some("otp") | Some("lifetime")) {
                        return "purchase.one_time".to_string();
                    }

                    if let Some(bt) = billing_type {
                        tracing::warn!(
                            billing_type = bt,
                            "creem checkout.completed: unrecognized billing_type, falling back to structural heuristics"
                        );
                    } else {
                        tracing::warn!(
                            "creem checkout.completed: billing_type absent in payload, falling back to structural heuristics"
                        );
                    }

                    if object.pointer("/subscription/id").and_then(|v| v.as_str()).is_some() {
                        tracing::warn!("creem checkout.completed: classified as subscription.created via /subscription/id heuristic");
                        return "subscription.created".to_string();
                    }

                    if object.get("checkout_id").and_then(|v| v.as_str()).is_some()
                        || object.get("order_id").and_then(|v| v.as_str()).is_some()
                        || object.pointer("/order/id").and_then(|v| v.as_str()).is_some()
                    {
                        tracing::warn!("creem checkout.completed: classified as purchase.one_time via order id heuristic");
                        return "purchase.one_time".to_string();
                    }

                    tracing::warn!("creem checkout.completed: no billing_type or structural heuristics matched, producing non-canonical event type");
                }

                match raw_event_type {
                    "subscription.created" => "subscription.created".to_string(),
                    "subscription.active" => "subscription.activated".to_string(),
                    "subscription.paid" => "subscription.activated".to_string(),
                    "subscription.trialing" => "subscription.trial_started".to_string(),
                    "subscription.past_due" => "subscription.grace_period".to_string(),
                    "subscription.paused" => "subscription.paused".to_string(),
                    "subscription.cancelled" | "subscription.canceled" => "subscription.cancelled".to_string(),
                    "subscription.expired" => "subscription.expired".to_string(),
                    "subscription.renewed" => "subscription.renewed".to_string(),
                    "subscription.scheduled_cancel" | "subscription.cancellation_scheduled" => "subscription.cancellation_scheduled".to_string(),
                    "order.created" => "payment.pending".to_string(),
                    "order.failed" | "payment.failed" => "payment.failed".to_string(),
                    "order.completed" => "subscription.activated".to_string(),
                    "one_time_product.purchased" => "purchase.one_time".to_string(),
                    "one_time_product.canceled" => "purchase.one_time_cancelled".to_string(),
                    "purchase.voided" => "payment.refunded".to_string(),
                    "subscription.pending_purchase_canceled" => "subscription.pending_purchase_cancelled".to_string(),
                    "refund.created" => "payment.refunded".to_string(),
                    "payment.partially_refunded" => "payment.partially_refunded".to_string(),
                    "dispute.created" => "dispute.created".to_string(),
                    "subscription.update" => "subscription.updated".to_string(),
                    "subscription.price_changed" => "subscription.price_changed".to_string(),
                    "subscription.price_change_updated" => "subscription.price_change_updated".to_string(),
                    "subscription.expired_voided" => "subscription.expired_voided".to_string(),
                    "subscription.deferred" => "subscription.deferred".to_string(),
                    "subscription.pause_scheduled" => "subscription.pause_scheduled".to_string(),
                    "subscription.price_step_up_consent_updated" => "subscription.price_step_up".to_string(),
                    _ => raw_event_type.to_string(),
                }
            }
        }
    }

    pub(crate) fn normalize_status(&self, raw_status: Option<&str>) -> NormalizedProviderStatus {
        let Some(s) = raw_status else { return NormalizedProviderStatus::Missing; };
        let cleaned = s.trim().to_ascii_lowercase();
        match cleaned.as_str() {
            "trialing" | "trial" => NormalizedProviderStatus::Known("trial".to_string()),
            "active" | "paid" | "completed" | "success" => NormalizedProviderStatus::Known("active".to_string()),
            "past_due" | "grace_period" => NormalizedProviderStatus::Known("past_due".to_string()),
            "cancelled" | "canceled" => NormalizedProviderStatus::Known("cancelled".to_string()),
            "expired" => NormalizedProviderStatus::Known("expired".to_string()),
            "on_hold" | "on-hold" => NormalizedProviderStatus::Known("on_hold".to_string()),
            "paused" => NormalizedProviderStatus::Known("paused".to_string()),
            "revoked" => NormalizedProviderStatus::Known("revoked".to_string()),
            "pending" => NormalizedProviderStatus::Known("pending".to_string()),
            _ => {
                tracing::warn!(raw_status = s, cleaned_status = %cleaned, provider = ?self, "unknown status ignored");
                NormalizedProviderStatus::Unknown(s.to_string())
            }
        }
    }

    pub(crate) fn extract_fields(
        &self,
        raw_event_type: &str,
        payload: &serde_json::Value,
    ) -> WebhookFields {
        let p = payload;
        match self {
            Self::GooglePlay => WebhookFields {
                subscription_id: p.pointer("/subscriptionNotification/subscriptionId")
                    .and_then(|v| v.as_str()).map(|s| s.to_string()),
                purchase_token: p.pointer("/subscriptionNotification/purchaseToken")
                    .and_then(|v| v.as_str()).map(|s| s.to_string())
                    .or_else(|| p.pointer("/oneTimeProductNotification/purchaseToken")
                        .and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .or_else(|| p.pointer("/voidedPurchaseNotification/purchaseToken")
                        .and_then(|v| v.as_str()).map(|s| s.to_string())),
                amount_cents: p.pointer("/oneTimeProductNotification/priceMicros")
                    .and_then(|v| v.as_i64()).map(|m| m / 10_000),
                auto_renewing: p.pointer("/subscriptionNotification/autoRenewing")
                    .and_then(|v| v.as_bool()),
                current_period_end: p.pointer("/subscriptionNotification/expiryTimeMillis")
                    .and_then(|v| v.as_i64())
                    .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                    .map(|dt| dt.to_rfc3339()),
                provider_transaction_id: p.pointer("/subscriptionNotification/orderId")
                    .and_then(|v| v.as_str()).map(|s| s.to_string())
                    .or_else(|| p.pointer("/voidedPurchaseNotification/orderId")
                        .and_then(|v| v.as_str()).map(|s| s.to_string())),
                provider_customer_id: None,
                product_id: p.pointer("/subscriptionNotification/subscriptionId")
                    .and_then(|v| v.as_str()).map(|s| s.to_string())
                    .or_else(|| p.pointer("/oneTimeProductNotification/productId")
                        .and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .or_else(|| p.pointer("/oneTimeProductNotification/sku")
                        .and_then(|v| v.as_str()).map(|s| s.to_string())),
                cancel_reason: p.pointer("/subscriptionNotification/cancelReason")
                    .and_then(|v| v.as_i64()).map(|c| c.to_string()),
                currency: None,
                status: None,
                google_subscription_state: None,
                google_cancellation_context: None,
                google_cancellation_feedback: None,
                google_new_price_cents: p.pointer("/subscriptionNotification/priceStepUpConsentDetails/priceMicros")
                    .and_then(|v| v.as_i64()).map(|m| m / 10_000),
                google_price_step_up_consent_deadline: p.pointer("/subscriptionNotification/priceStepUpConsentDetails/consentDeadlineTimeMillis")
                    .and_then(|v| v.as_i64())
                    .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                    .map(|dt| dt.to_rfc3339()),
                google_pending_price_change_new_price_cents: None,
                google_pending_price_change_currency: None,
                google_pending_price_change_mode: None,
                google_pending_price_change_state: None,
                google_pending_price_change_expected_at: None,
            },
            Self::Creem => {
                let obj = p.get("object").unwrap_or(&serde_json::Value::Null);

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
                let object_transaction_id = obj.get("transaction")
                    .and_then(|v| {
                        v.as_str()
                            .or_else(|| v.get("id").and_then(|id| id.as_str()))
                    })
                    .map(|s| s.to_string())
                    .or_else(|| obj.get("order")
                        .and_then(|v| v.get("transaction"))
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

                let normalized_event_type = self.canonical_event_type(
                    raw_event_type,
                    p,
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

                // Prefer actual cash collected when Creem sends a transaction object.
                // Trial invoices can have amount_paid = 0 while amount is the recurring list price.
                // Refunds/disputes use `refund_amount` + a `transaction` (not `last_transaction`)
                // object, so those paths are covered as fallbacks.
                let amount_cents = obj.get("last_transaction")
                    .and_then(|v| v.get("amount_paid").or_else(|| v.get("amount")))
                    .and_then(|v| v.as_i64())
                    .or_else(|| obj.get("refund_amount").and_then(|v| v.as_i64()))
                    .or_else(|| obj.get("transaction")
                        .and_then(|v| v.get("amount_paid").or_else(|| v.get("amount")))
                        .and_then(|v| v.as_i64()))
                    .or_else(|| obj.get("order")
                        .and_then(|v| v.get("amount"))
                        .and_then(|v| v.as_i64()))
                    .or_else(|| obj.get("product")
                        .and_then(|v| v.get("price"))
                        .and_then(|v| v.as_i64()))
                    .or_else(|| obj.get("amount").and_then(|v| v.as_i64()));
                // Currency mirrors the amount fallback chain: refunds use `refund_currency`
                // or `transaction.currency`, disputes use top-level `currency`, checkout uses
                // `order.currency`. `product.currency` covers subscription/checkout events.
                let currency = obj.get("product")
                    .and_then(|v| v.get("currency"))
                    .and_then(|v| v.as_str())
                    .or_else(|| obj.get("transaction")
                        .and_then(|v| v.get("currency"))
                        .and_then(|v| v.as_str()))
                    .or_else(|| obj.get("order")
                        .and_then(|v| v.get("currency"))
                        .and_then(|v| v.as_str()))
                    .or_else(|| obj.get("refund_currency").and_then(|v| v.as_str()))
                    .or_else(|| obj.get("currency").and_then(|v| v.as_str()))
                    .map(|s| s.to_string());

                // Extract purchase_token (checkout_id for OTP, order_id for refunds)
                let purchase_token = match normalized_event_type.as_str() {
                    "purchase.one_time" | "purchase.one_time_refunded" => object_checkout_id.clone()
                        .or_else(|| object_order_id.clone())
                        .or_else(|| object_id.clone()),
                    "payment.refunded" => object_order_id.clone()
                        .or_else(|| object_checkout_id.clone())
                        .or_else(|| object_subscription_id.clone())
                        .or_else(|| object_id.clone()),
                    "payment.partially_refunded" => object_order_id.clone()
                        .or_else(|| object_checkout_id.clone())
                        .or_else(|| object_id.clone()),
                    _ => None,
                };

                let provider_transaction_id = match normalized_event_type.as_str() {
                    "purchase.one_time" => object_order_id
                        .clone()
                        .or_else(|| object_checkout_id.clone())
                        .or_else(|| object_id.clone()),
                    "payment.refunded" | "payment.partially_refunded" => object_transaction_id
                        .clone()
                        .or_else(|| obj.get("last_transaction_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())),
                    _ => obj.get("last_transaction_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                };

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
                    currency,
                    status,
                    google_subscription_state: None,
                    google_cancellation_context: None,
                    google_cancellation_feedback: None,
                    google_new_price_cents: None,
                    google_price_step_up_consent_deadline: None,
                    google_pending_price_change_new_price_cents: None,
                    google_pending_price_change_currency: None,
                    google_pending_price_change_mode: None,
                    google_pending_price_change_state: None,
                    google_pending_price_change_expected_at: None,
                }
            }
        }
    }
}

fn sanitize_google_play_payload_for_log(payload: &serde_json::Value) -> String {
    let mut sanitized = payload.clone();

    for pointer in [
        "/subscriptionNotification/purchaseToken",
        "/oneTimeProductNotification/purchaseToken",
        "/voidedPurchaseNotification/purchaseToken",
    ] {
        if let Some(value) = sanitized.pointer_mut(pointer) {
            if let Some(token) = value.as_str() {
                *value = serde_json::Value::String(diagnostic_hash(token));
            }
        }
    }

    let payload = serde_json::to_string(&sanitized).unwrap_or_else(|_| "{}".to_string());
    crate::utils::scrub_email(&payload)
}


fn decode_base64_flexible(input: &str) -> Result<Vec<u8>, String> {
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(input) {
        return Ok(decoded);
    }
    if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE.decode(input) {
        return Ok(decoded);
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .map_err(|e| e.to_string())
}

fn decode_google_play_payload(
    payload: &serde_json::Value,
    headers: &HeaderMap,
) -> Result<(serde_json::Value, Option<String>), BridgeError> {
    if payload.get("message").is_some() {
        let message_data = payload["message"]["data"].as_str().ok_or_else(|| {
            error!(provider = "google_play", "Missing message.data in webhook");
            BridgeError::WebhookError("Missing message.data field".to_string())
        })?;

        let decoded_message = decode_base64_flexible(message_data)
            .map_err(|e| BridgeError::WebhookError(format!("Invalid message.data: {}", e)))?;

        let google_play_event: serde_json::Value = serde_json::from_slice(&decoded_message).map_err(|e| {
            error!(error = %e, provider = "google_play", "Failed to parse message.data payload");
            BridgeError::WebhookError(format!("Invalid Google Play message.data payload: {}", e))
        })?;

        let message_id = payload["message"]["messageId"]
            .as_str()
            .or_else(|| payload["message"]["message_id"].as_str())
            .map(|s| s.to_string());

        return Ok((google_play_event, message_id));
    }

    let message_id = headers
        .get("x-goog-pubsub-message-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    Ok((payload.clone(), message_id))
}


#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{normalize_creem_event, normalize_google_play_event};

    #[test]
    fn normalizes_google_subscription_ingress_fields_without_changing_identity() {
        let payload = json!({
            "eventTimeMillis": "1778936707956",
            "subscriptionNotification": {
                "notificationType": 2,
                "subscriptionId": "premium_monthly",
                "purchaseToken": "purchase_token",
                "orderId": "GPA.1234-5678"
            }
        });

        let event = normalize_google_play_event(
            payload.clone(),
            Some("pubsub-message-id".to_string()),
        )
        .unwrap();

        assert_eq!(event.provider, "google_play");
        assert_eq!(event.provider_event_id, "pubsub-message-id");
        assert_eq!(event.raw_event_type, "SUBSCRIPTION_RENEWED");
        assert_eq!(event.occurred_at_ms, Some(1778936707956));
        assert_eq!(event.subscription_id.as_deref(), Some("premium_monthly"));
        assert_eq!(event.purchase_token.as_deref(), Some("purchase_token"));
        assert_eq!(event.payload, payload);
    }

    #[test]
    fn normalizes_google_one_time_ingress_fields_without_using_token_as_transaction_id() {
        let payload = json!({
            "eventId": "google-event-id",
            "eventTimeMillis": 1778936707956_i64,
            "oneTimeProductNotification": {
                "notificationType": "1",
                "productId": "lifetime_access",
                "purchaseToken": "purchase_token"
            }
        });

        let event = normalize_google_play_event(payload, None).unwrap();

        assert_eq!(event.provider_event_id, "google-event-id");
        assert_eq!(event.raw_event_type, "ONE_TIME_PRODUCT_PURCHASED");
        assert_eq!(event.subscription_id, None);
        assert_eq!(event.purchase_token.as_deref(), Some("purchase_token"));
    }

    #[test]
    fn normalizes_creem_ingress_fields_without_changing_persisted_values() {
        let payload = json!({
            "id": "evt_creem",
            "eventType": "subscription.active",
            "createdAt": "2026-06-30T10:00:00Z",
            "object": {
                "id": "sub_creem",
                "subscription_id": "sub_creem",
                "checkout_id": "ch_creem",
                "product": {
                    "id": "prod_creem"
                },
                "last_transaction_id": "txn_creem"
            }
        });

        let event = normalize_creem_event(payload.clone()).unwrap();

        assert_eq!(event.provider, "creem");
        assert_eq!(event.provider_event_id, "evt_creem");
        assert_eq!(event.raw_event_type, "subscription.active");
        assert_eq!(event.occurred_at_ms, Some(1782813600000));
        assert_eq!(event.subscription_id.as_deref(), Some("sub_creem"));
        assert_eq!(event.purchase_token.as_deref(), Some("ch_creem"));
        assert_eq!(event.payload, payload);
    }

    #[test]
    fn decodes_wrapped_google_play_pubsub_payload() {
        use base64::Engine as _;
        use axum::http::HeaderMap;
        use super::decode_google_play_payload;

        let headers = HeaderMap::new();
        let google_event = json!({
            "version": "1.0",
            "packageName": "com.hiha.fe",
            "eventTimeMillis": "1778936707956",
            "testNotification": { "version": "1.0" }
        });
        let payload = json!({
            "message": {
                "messageId": "wrapped-message-id",
                "data": base64::engine::general_purpose::STANDARD.encode(google_event.to_string())
            },
            "subscription": "projects/play/subscriptions/play-sub-dev"
        });

        let (decoded, message_id) = decode_google_play_payload(&payload, &headers).unwrap();

        assert_eq!(message_id.as_deref(), Some("wrapped-message-id"));
        assert_eq!(decoded["packageName"].as_str(), Some("com.hiha.fe"));
        assert!(decoded.get("testNotification").is_some());
    }

    #[test]
    fn accepts_unwrapped_google_play_pubsub_payload() {
        use axum::http::{HeaderMap, HeaderValue};
        use super::decode_google_play_payload;

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-goog-pubsub-message-id",
            HeaderValue::from_static("unwrapped-message-id"),
        );
        let payload = json!({
            "version": "1.0",
            "packageName": "com.hiha.fe",
            "eventTimeMillis": "1778936707956",
            "testNotification": { "version": "1.0" }
        });

        let (decoded, message_id) = decode_google_play_payload(&payload, &headers).unwrap();

        assert_eq!(message_id.as_deref(), Some("unwrapped-message-id"));
        assert_eq!(decoded["packageName"].as_str(), Some("com.hiha.fe"));
        assert!(decoded.get("testNotification").is_some());
    }
}
