use crate::{
    db::webhooks::WebhookProvider,
    error::BridgeError,
    ports::{
        TransactionOutcome, WebhookProcessingLookupRepository, WebhookProcessingTransactionRepository,
    },
    services::google_play::subscription_lifecycle::GooglePlayLifecycleOutcome,
    webhooks::processor::WebhookFields,
};
use uuid::Uuid;

pub async fn handle_otp_purchased<R: WebhookProcessingTransactionRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProvider,
    fields: &WebhookFields,
    external_user_id: Option<&str>,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(user_id) = external_user_id
    else {
        return Ok(None);
    };

    let txn_id = fields
        .purchase_token
        .as_deref()
        .or(fields.provider_transaction_id.as_deref())
        .unwrap_or(&webhook.provider_webhook_id);

    repo
        .with_transaction(|tx| {
            Box::pin(async {
                repo.record_payment_tx(
                    tx,
                    app_id,
                    user_id,
                    &webhook.provider,
                    txn_id,
                    fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()),
                    fields.amount_cents.unwrap_or(0),
                    "success",
                )
                .await?;

                Ok(TransactionOutcome::Commit(()))
            })
        })
        .await?;

    let _ = timestamp_epoch_ms;

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: None,
        callback_event_type: Some("purchase.one_time".to_string()),
        callback_status_override: Some("completed".to_string()),
        callback_revocation_reason_override: None,
        callback_cancellation_mode_override: None,
    }))
}

pub async fn handle_otp_cancelled<
    R: WebhookProcessingTransactionRepository + WebhookProcessingLookupRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProvider,
    fields: &WebhookFields,
    external_user_id: Option<&str>,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(user_id) = external_user_id
    else {
        return Ok(None);
    };

    let token = fields
        .purchase_token
        .as_deref()
        .or(fields.provider_transaction_id.as_deref())
        .unwrap_or("");

    let existing = repo.get_payment_status(app_id, token).await?;
    if matches!(existing.as_deref(), Some("refunded") | Some("cancelled")) {
        return Ok(None);
    }

    repo
        .with_transaction(|tx| {
            Box::pin(async {
                repo.record_payment_tx(
                    tx,
                    app_id,
                    user_id,
                    &webhook.provider,
                    token,
                    fields.subscription_id.as_deref().or(webhook.subscription_id.as_deref()),
                    fields.amount_cents.unwrap_or(0),
                    "cancelled",
                )
                .await?;

                Ok(TransactionOutcome::Commit(()))
            })
        })
        .await?;

    let _ = timestamp_epoch_ms;

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: None,
        callback_event_type: Some("purchase.one_time".to_string()),
        callback_status_override: Some("cancelled".to_string()),
        callback_revocation_reason_override: None,
        callback_cancellation_mode_override: None,
    }))
}
