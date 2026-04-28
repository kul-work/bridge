use crate::{
    error::BridgeError,
    ports::{
        SubscriptionWebhookTransition, WebhookProcessingLookupRepository,
        WebhookProcessingMutationRepository, WebhookProviderSnapshot,
        WebhookSubscriptionSnapshot,
    },
    webhooks::processor::WebhookFields,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct GooglePlayLifecycleOutcome {
    pub canonical_subscription: Option<WebhookSubscriptionSnapshot>,
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
    webhook: &'a WebhookProviderSnapshot,
) -> Option<&'a str> {
    fields
        .subscription_id
        .as_deref()
        .or(webhook.subscription_id.as_deref())
}

#[allow(clippy::too_many_arguments)]
async fn apply_transition_with_outcome<
    R: WebhookProcessingMutationRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    subscription_id: &str,
    timestamp_epoch_ms: i64,
    transition: SubscriptionWebhookTransition,
    mut outcome: GooglePlayLifecycleOutcome,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let updated = repo
        .apply_subscription_transition(
            app_id,
            external_user_id,
            provider,
            subscription_id,
            timestamp_epoch_ms,
            transition,
        )
        .await?;

    let Some(subscription) = updated else {
        return Ok(None);
    };

    outcome.canonical_subscription = Some(subscription.into());
    Ok(Some(outcome))
}

pub async fn handle_subscription_revoked<
    R: WebhookProcessingLookupRepository + WebhookProcessingMutationRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    external_user_id: &str,
    webhook: &WebhookProviderSnapshot,
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
        if repo.get_payment_status_for_provider(app_id, &webhook.provider, token).await?.as_deref() == Some("refunded") {
            return Ok(None);
        }
    }

    let revocation_reason = fields
        .cancel_reason
        .clone()
        .or_else(|| fields.google_cancellation_context.clone())
        .unwrap_or_else(|| "unknown".to_string());

    apply_transition_with_outcome(
        repo,
        app_id,
        external_user_id,
        &webhook.provider,
        subscription_id,
        timestamp_epoch_ms,
        SubscriptionWebhookTransition::Revoked {
            revocation_reason: Some(revocation_reason.clone()),
        },
        GooglePlayLifecycleOutcome {
            callback_event_type: Some("subscription.revoked".to_string()),
            callback_status_override: Some("revoked".to_string()),
            callback_revocation_reason_override: Some(revocation_reason),
            ..Default::default()
        },
    )
    .await
}

pub async fn handle_subscription_resumed<
    R: WebhookProcessingLookupRepository + WebhookProcessingMutationRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    external_user_id: &str,
    webhook: &WebhookProviderSnapshot,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    let Some(subscription) = repo
        .get_subscription_by_sub_id_and_user_for_provider(app_id, &webhook.provider, subscription_id, external_user_id)
        .await?
    else {
        return Ok(None);
    };

    if subscription.status != "paused" && subscription.status != "cancelled" {
        return Ok(None);
    }

    let period_end = fields
        .current_period_end
        .as_deref()
        .and_then(parse_rfc3339_utc);

    apply_transition_with_outcome(
        repo,
        app_id,
        external_user_id,
        &webhook.provider,
        subscription_id,
        timestamp_epoch_ms,
        SubscriptionWebhookTransition::Resumed {
            current_period_end: period_end,
        },
        GooglePlayLifecycleOutcome {
            callback_event_type: Some("subscription.resumed".to_string()),
            callback_status_override: Some("active".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub async fn handle_subscription_cancelled_with_context<
    R: WebhookProcessingLookupRepository + WebhookProcessingMutationRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    external_user_id: &str,
    webhook: &WebhookProviderSnapshot,
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

    apply_transition_with_outcome(
        repo,
        app_id,
        external_user_id,
        &webhook.provider,
        subscription_id,
        timestamp_epoch_ms,
        SubscriptionWebhookTransition::Cancelled {
            current_period_end: cancel_period_end,
            google_cancellation_context: fields.google_cancellation_context.clone(),
            google_cancellation_feedback: fields.google_cancellation_feedback.clone(),
        },
        GooglePlayLifecycleOutcome {
            callback_event_type: Some("subscription.cancelled".to_string()),
            callback_status_override: Some("cancelled".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub async fn handle_subscription_cancellation_scheduled<
    R: WebhookProcessingLookupRepository + WebhookProcessingMutationRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    external_user_id: &str,
    webhook: &WebhookProviderSnapshot,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    apply_transition_with_outcome(
        repo,
        app_id,
        external_user_id,
        &webhook.provider,
        subscription_id,
        timestamp_epoch_ms,
        SubscriptionWebhookTransition::CancellationScheduled {
            google_cancellation_context: fields.google_cancellation_context.clone(),
            google_cancellation_feedback: fields.google_cancellation_feedback.clone(),
        },
        GooglePlayLifecycleOutcome {
            callback_event_type: Some("subscription.cancelled".to_string()),
            callback_status_override: Some("cancelled".to_string()),
            callback_cancellation_mode_override: Some("scheduled".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub async fn handle_price_step_up_consent_required<
    R: WebhookProcessingLookupRepository + WebhookProcessingMutationRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    external_user_id: &str,
    webhook: &WebhookProviderSnapshot,
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

    apply_transition_with_outcome(
        repo,
        app_id,
        external_user_id,
        &webhook.provider,
        subscription_id,
        timestamp_epoch_ms,
        SubscriptionWebhookTransition::PriceStepUp {
            google_new_price_cents: fields.google_new_price_cents,
            google_price_step_up_consent_deadline: deadline,
        },
        GooglePlayLifecycleOutcome::default(),
    )
    .await
}

pub async fn handle_subscription_pending_purchase_cancelled<
    R: WebhookProcessingLookupRepository + WebhookProcessingMutationRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    external_user_id: &str,
    webhook: &WebhookProviderSnapshot,
    fields: &WebhookFields,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(subscription_id) = subscription_id_for_event(fields, webhook) else {
        return Ok(None);
    };

    apply_transition_with_outcome(
        repo,
        app_id,
        external_user_id,
        &webhook.provider,
        subscription_id,
        timestamp_epoch_ms,
        SubscriptionWebhookTransition::PendingPurchaseCancelled,
        GooglePlayLifecycleOutcome {
            callback_event_type: Some("subscription.cancelled".to_string()),
            callback_status_override: Some("cancelled".to_string()),
            ..Default::default()
        },
    )
    .await
}
