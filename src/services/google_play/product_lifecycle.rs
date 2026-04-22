use crate::{
    error::BridgeError,
    ports::{
        WebhookPaymentRecordRequest, WebhookProcessingLookupRepository,
        WebhookProcessingTransactionRepository, WebhookProviderSnapshot,
    },
    services::google_play::subscription_lifecycle::GooglePlayLifecycleOutcome,
    webhooks::processor::WebhookFields,
};
use uuid::Uuid;

pub async fn handle_otp_purchased<R: WebhookProcessingTransactionRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProviderSnapshot,
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
        .record_webhook_payment(WebhookPaymentRecordRequest {
            app_id,
            external_user_id: user_id,
            provider: &webhook.provider,
            provider_transaction_id: txn_id,
            subscription_id: None,
            product_id: fields.product_id.as_deref(),
            amount_cents: fields.amount_cents.unwrap_or(0),
            status: "success",
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
    webhook: &WebhookProviderSnapshot,
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

    let existing = repo.get_payment_status_for_provider(app_id, &webhook.provider, token).await?;
    if matches!(existing.as_deref(), Some("refunded") | Some("cancelled")) {
        return Ok(None);
    }

    repo
        .record_webhook_payment(WebhookPaymentRecordRequest {
            app_id,
            external_user_id: user_id,
            provider: &webhook.provider,
            provider_transaction_id: token,
            subscription_id: None,
            product_id: fields.product_id.as_deref(),
            amount_cents: fields.amount_cents.unwrap_or(0),
            status: "cancelled",
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
