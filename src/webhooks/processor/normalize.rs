
pub(super) fn callback_status_for_event(event_type: &str) -> Option<String> {
    match event_type {
        "subscription.activated" | "subscription.resumed" | "subscription.pause_scheduled" | "subscription.price_changed" | "subscription.price_change_updated" => {
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
