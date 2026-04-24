pub(super) fn normalize_event_type_with_payload(
    provider: &str,
    event_type: &str,
    payload: Option<&serde_json::Value>,
) -> String {
    if provider == "creem" && event_type == "checkout.completed" {
        if let Some(payload) = payload {
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
        } else {
            tracing::warn!("creem checkout.completed: payload missing, cannot determine canonical event type");
        }
    }

    normalize_event_type(provider, event_type)
}

/// Normalize provider-specific event type to canonical format
/// Maps provider events per architecture doc section 3.4
pub(super) fn normalize_event_type(provider: &str, event_type: &str) -> String {
    match provider {
        "google_play" => match event_type {
            // Google Play subscription notifications
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
            "ONE_TIME_PRODUCT_REFUNDED" => "payment.refunded".to_string(),
            "ONE_TIME_PRODUCT_CANCELED" => "purchase.one_time_cancelled".to_string(),
            "VOIDED_PURCHASE" => "payment.refunded".to_string(),
            _ => format!("google_play.{}", event_type),
        },
        "creem" => match event_type {
            // Creem subscription events
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
            _ => event_type.to_string(),
        },
        "coinbase" => match event_type {
            "charge:confirmed" => "charge.confirmed".to_string(),
            "charge:failed" => "charge.failed".to_string(),
            _ => event_type.to_string(),
        },
        _ => event_type.to_string(),
    }
}

/// Normalize raw provider status to canonical Bridge status
pub(super) fn normalize_status(raw_status: Option<&str>) -> String {
    let Some(s) = raw_status else { return "pending".to_string(); };
    match s.trim().to_ascii_lowercase().as_str() {
        "trialing" | "trial" => "trial".to_string(),
        "active" | "paid" | "completed" | "success" => "active".to_string(),
        "past_due" | "grace_period" => "past_due".to_string(),
        "cancelled" | "canceled" => "cancelled".to_string(),
        "expired" => "expired".to_string(),
        "on_hold" | "on-hold" => "on_hold".to_string(),
        "paused" => "paused".to_string(),
        "revoked" => "revoked".to_string(),
        "pending" => "pending".to_string(),
        _ => s.to_string(),
    }
}

pub(super) fn callback_status_for_event(event_type: &str) -> Option<String> {
    match event_type {
        "subscription.activated" | "subscription.resumed" | "subscription.pause_scheduled" => {
            Some("active".to_string())
        }
        "subscription.grace_period" => Some("past_due".to_string()),
        "subscription.revoked" => Some("revoked".to_string()),
        "subscription.on_hold" => Some("on_hold".to_string()),
        "subscription.paused" => Some("paused".to_string()),
        "subscription.expired" => Some("expired".to_string()),
        "subscription.cancelled" => Some("cancelled".to_string()),
        _ => None,
    }
}

/// Map normalized status to canonical callback event type
pub(super) fn status_to_canonical_event(normalized_status: &str) -> Option<String> {
    match normalized_status {
        "active" | "trial" => Some("subscription.activated".to_string()),
        "past_due" => Some("subscription.grace_period".to_string()),
        "on_hold" => Some("subscription.on_hold".to_string()),
        "paused" => Some("subscription.paused".to_string()),
        "expired" => Some("subscription.expired".to_string()),
        "cancelled" => Some("subscription.cancelled".to_string()),
        "revoked" => Some("subscription.revoked".to_string()),
        _ => None,
    }
}

pub(super) fn parse_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}
