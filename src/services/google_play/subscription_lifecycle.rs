use crate::{
    db::{
        self,
        subscriptions::SubscriptionWebhookTransition,
        webhooks::WebhookProvider,
    },
    error::BridgeError,
    ports::WebhookProcessingRepository,
    webhooks::processor::WebhookFields,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct GooglePlayLifecycleOutcome {
    pub canonical_subscription: Option<db::subscriptions::Subscription>,
    pub callback_event_type: Option<String>,
    pub callback_status_override: Option<String>,
    pub callback_revocation_reason_override: Option<String>,
    pub callback_cancellation_mode_override: Option<String>,
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn subscription_id_for_event<'a>(
    fields: &'a WebhookFields,
    webhook: &'a WebhookProvider,
) -> Option<&'a str> {
    fields
        .subscription_id
        .as_deref()
        .or(webhook.subscription_id.as_deref())
}

fn outcome_with_subscription(subscription: db::subscriptions::Subscription) -> GooglePlayLifecycleOutcome {
    GooglePlayLifecycleOutcome {
        canonical_subscription: Some(subscription),
        ..Default::default()
    }
}

pub async fn handle_subscription_revoked<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProvider,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    if let Some(token) = fields
        .purchase_token
        .as_deref()
        .or(webhook.purchase_token.as_deref())
    {
        if repo.get_payment_status(app_id, token).await?.as_deref() == Some("refunded") {
            return Ok(None);
        }
    }

    let revocation_reason = fields
        .cancel_reason
        .clone()
        .or_else(|| fields.google_cancellation_context.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let updated = repo
        .apply_subscription_transition(
            app_id,
            subscription_id,
            timestamp_epoch_ms,
            SubscriptionWebhookTransition::Revoked {
                revocation_reason: Some(revocation_reason.clone()),
            },
        )
        .await?;

    let Some(subscription) = updated else {
        return Ok(None);
    };

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: Some(subscription),
        callback_event_type: Some("subscription.revoked".to_string()),
        callback_status_override: Some("revoked".to_string()),
        callback_revocation_reason_override: Some(revocation_reason),
        callback_cancellation_mode_override: None,
    }))
}

pub async fn handle_subscription_resumed<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProvider,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    let Some(subscription) = repo
        .get_subscription_by_sub_id(app_id, subscription_id)
        .await?
    else {
        return Ok(None);
    };

    if subscription.status != "paused" {
        return Ok(None);
    }

    let updated = repo
        .apply_subscription_transition(
            app_id,
            subscription_id,
            timestamp_epoch_ms,
            SubscriptionWebhookTransition::Resumed,
        )
        .await?;

    let Some(subscription) = updated else {
        return Ok(None);
    };

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: Some(subscription),
        callback_event_type: Some("subscription.resumed".to_string()),
        callback_status_override: Some("active".to_string()),
        callback_revocation_reason_override: None,
        callback_cancellation_mode_override: None,
    }))
}

pub async fn handle_subscription_cancelled_with_context<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProvider,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    let cancel_period_end = fields
        .current_period_end
        .as_deref()
        .and_then(parse_rfc3339_utc);

    let updated = repo
        .apply_subscription_transition(
            app_id,
            subscription_id,
            timestamp_epoch_ms,
            SubscriptionWebhookTransition::Cancelled {
                current_period_end: cancel_period_end,
                google_cancellation_context: fields.google_cancellation_context.clone(),
                google_cancellation_feedback: fields.google_cancellation_feedback.clone(),
            },
        )
        .await?;

    let Some(subscription) = updated else {
        return Ok(None);
    };

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: Some(subscription),
        callback_event_type: Some("subscription.cancelled".to_string()),
        callback_status_override: Some("cancelled".to_string()),
        callback_revocation_reason_override: None,
        callback_cancellation_mode_override: None,
    }))
}

pub async fn handle_subscription_cancellation_scheduled<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProvider,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    let updated = repo
        .apply_subscription_transition(
            app_id,
            subscription_id,
            timestamp_epoch_ms,
            SubscriptionWebhookTransition::CancellationScheduled {
                google_cancellation_context: fields.google_cancellation_context.clone(),
                google_cancellation_feedback: fields.google_cancellation_feedback.clone(),
            },
        )
        .await?;

    let Some(subscription) = updated else {
        return Ok(None);
    };

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: Some(subscription),
        callback_event_type: Some("subscription.cancelled".to_string()),
        callback_status_override: Some("cancelled".to_string()),
        callback_revocation_reason_override: None,
        callback_cancellation_mode_override: Some("scheduled".to_string()),
    }))
}

pub async fn handle_price_step_up_consent_required<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProvider,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    let deadline = fields
        .google_price_step_up_consent_deadline
        .as_deref()
        .and_then(parse_rfc3339_utc);

    let updated = repo
        .apply_subscription_transition(
            app_id,
            subscription_id,
            timestamp_epoch_ms,
            SubscriptionWebhookTransition::PriceStepUp {
                google_new_price_cents: fields.google_new_price_cents,
                google_price_step_up_consent_deadline: deadline,
            },
        )
        .await?;

    let Some(subscription) = updated else {
        return Ok(None);
    };

    Ok(Some(outcome_with_subscription(subscription)))
}

pub async fn handle_subscription_pending_purchase_cancelled<R: WebhookProcessingRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProvider,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    let updated = repo
        .apply_subscription_transition(
            app_id,
            subscription_id,
            timestamp_epoch_ms,
            SubscriptionWebhookTransition::PendingPurchaseCancelled,
        )
        .await?;

    let Some(subscription) = updated else {
        return Ok(None);
    };

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: Some(subscription),
        callback_event_type: Some("subscription.cancelled".to_string()),
        callback_status_override: Some("cancelled".to_string()),
        callback_revocation_reason_override: None,
        callback_cancellation_mode_override: None,
    }))
}
