use serde::Serialize;
use uuid::Uuid;

use crate::config::MAX_PAGINATION_LIMIT;
use crate::db::subscriptions::Subscription;
use crate::error::BridgeError;
use crate::ports::SubscriptionReadRepository;

pub struct SubscriptionStatusInput<'a> {
    pub app_id: Uuid,
    pub external_user_id: &'a str,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionStatusSnapshot {
    pub is_premium: bool,
    pub subscription_id: Option<String>,
    pub provider: Option<String>,
    pub status: Option<String>,
    pub current_period_end: Option<String>,
    pub auto_renewing: Option<bool>,
    pub payment_failure_notification: bool,
    pub revoked_at: Option<String>,
    pub revocation_reason: Option<String>,
    pub google_requires_price_step_up_consent: Option<bool>,
    pub google_new_price_cents: Option<i32>,
    pub google_price_step_up_consent_deadline: Option<String>,
    pub google_pause_scheduled_at: Option<String>,
    pub google_deferred_until: Option<String>,
    pub last_event_time: Option<i64>,
}

pub async fn get_subscription_status_snapshot<R: SubscriptionReadRepository + ?Sized>(
    repo: &R,
    input: SubscriptionStatusInput<'_>,
) -> Result<SubscriptionStatusSnapshot, BridgeError> {
    let external_user_id = input.external_user_id.trim();
    if external_user_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "external_user_id is required".to_string(),
        ));
    }

    let subscriptions = repo.get_user_subscriptions(
        input.app_id,
        external_user_id,
        MAX_PAGINATION_LIMIT,
        0,
    )
    .await?;

    let is_premium = subscriptions.iter().any(subscription_is_premium);
    Ok(match select_subscription_for_snapshot(&subscriptions) {
        Some(sub) => SubscriptionStatusSnapshot {
            is_premium,
            subscription_id: Some(sub.subscription_id.clone()),
            provider: Some(sub.provider.clone()),
            status: Some(sub.status.clone()),
            current_period_end: sub.current_period_end.map(|d| d.to_rfc3339()),
            auto_renewing: sub.auto_renewing,
            payment_failure_notification: sub.payment_failure_notification,
            revoked_at: sub.revoked_at.map(|d| d.to_rfc3339()),
            revocation_reason: sub.revocation_reason.clone(),
            google_requires_price_step_up_consent: sub.google_requires_price_step_up_consent,
            google_new_price_cents: sub.google_new_price_cents,
            google_price_step_up_consent_deadline: sub.google_price_step_up_consent_deadline.map(|d| d.to_rfc3339()),
            google_pause_scheduled_at: sub.google_pause_scheduled_at.map(|d| d.to_rfc3339()),
            google_deferred_until: sub.google_deferred_until.map(|d| d.to_rfc3339()),
            last_event_time: Some(sub.last_event_time),
        },
        None => SubscriptionStatusSnapshot {
            is_premium: false,
            subscription_id: None,
            provider: None,
            status: None,
            current_period_end: None,
            auto_renewing: None,
            payment_failure_notification: false,
            revoked_at: None,
            revocation_reason: None,
            google_requires_price_step_up_consent: None,
            google_new_price_cents: None,
            google_price_step_up_consent_deadline: None,
            google_pause_scheduled_at: None,
            google_deferred_until: None,
            last_event_time: None,
        },
    })
}

fn subscription_is_premium(sub: &Subscription) -> bool {
    matches!(sub.status.as_str(), "active" | "trial" | "past_due")
}

fn snapshot_status_rank(status: &str) -> i32 {
    match status {
        "active" => 0,
        "trial" => 1,
        "past_due" => 2,
        "pending" => 3,
        "on_hold" => 4,
        "paused" => 5,
        "cancelled" => 6,
        "expired" => 7,
        "revoked" => 8,
        _ => 9,
    }
}

fn select_subscription_for_snapshot(subscriptions: &[Subscription]) -> Option<&Subscription> {
    subscriptions.iter().min_by(|left, right| {
        snapshot_status_rank(&left.status)
            .cmp(&snapshot_status_rank(&right.status))
            .then_with(|| right.last_event_time.cmp(&left.last_event_time))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
    })
}
