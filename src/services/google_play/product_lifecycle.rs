use crate::{
    error::BridgeError,
    ports::{
        WebhookPaymentRecordRequest, WebhookProcessingLookupRepository,
        WebhookProcessingMutationRepository, WebhookProcessingTransactionRepository,
        WebhookProviderSnapshot,
    },
    services::google_play::subscription_lifecycle::GooglePlayLifecycleOutcome,
    webhooks::processor::WebhookFields,
};
use uuid::Uuid;

pub async fn handle_otp_purchased<
    R: WebhookProcessingLookupRepository
        + WebhookProcessingMutationRepository
        + WebhookProcessingTransactionRepository
        + ?Sized,
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

    if let Some(purchase_token) = fields.purchase_token.as_deref().or(webhook.purchase_token.as_deref()) {
        if repo.get_payment_status_for_provider(app_id, &webhook.provider, purchase_token).await?.is_some() {
            repo.update_payment_status_for_provider(app_id, &webhook.provider, purchase_token, "success").await?;
            let _ = timestamp_epoch_ms;

            return Ok(Some(GooglePlayLifecycleOutcome {
                canonical_subscription: None,
                callback_event_type: Some("purchase.one_time".to_string()),
                callback_status_override: Some("completed".to_string()),
                callback_revocation_reason_override: None,
                callback_cancellation_mode_override: None,
            }));
        }
    }

    let txn_id = fields
        .provider_transaction_id
        .as_deref()
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
            currency: fields.currency.as_deref(),
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
    R: WebhookProcessingMutationRepository + WebhookProcessingLookupRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProviderSnapshot,
    fields: &WebhookFields,
    external_user_id: Option<&str>,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(_user_id) = external_user_id
    else {
        return Ok(None);
    };

    let token = fields
        .purchase_token
        .as_deref()
        .or(fields.provider_transaction_id.as_deref())
        .unwrap_or("");
    if token.is_empty() {
        return Ok(None);
    }

    let existing = repo.get_payment_status_for_provider(app_id, &webhook.provider, token).await?;
    if matches!(existing.as_deref(), Some("refunded") | Some("cancelled")) {
        return Ok(None);
    }

    repo.update_payment_status_for_provider(app_id, &webhook.provider, token, "cancelled").await?;

    let _ = timestamp_epoch_ms;

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: None,
        callback_event_type: Some("purchase.one_time".to_string()),
        callback_status_override: Some("cancelled".to_string()),
        callback_revocation_reason_override: None,
        callback_cancellation_mode_override: None,
    }))
}

pub async fn handle_otp_refunded<
    R: WebhookProcessingMutationRepository + WebhookProcessingLookupRepository + ?Sized,
>(
    repo: &R,
    app_id: Uuid,
    webhook: &WebhookProviderSnapshot,
    fields: &WebhookFields,
    external_user_id: Option<&str>,
    timestamp_epoch_ms: i64,
) -> Result<Option<GooglePlayLifecycleOutcome>, BridgeError> {
    let Some(_user_id) = external_user_id
    else {
        return Ok(None);
    };

    let token = fields
        .purchase_token
        .as_deref()
        .or(fields.provider_transaction_id.as_deref())
        .unwrap_or("");
    if token.is_empty() {
        return Ok(None);
    }

    let existing = repo.get_payment_status_for_provider(app_id, &webhook.provider, token).await?;
    if existing.as_deref() == Some("refunded") {
        return Ok(None);
    }

    repo.update_payment_status_for_provider(app_id, &webhook.provider, token, "refunded").await?;

    let _ = timestamp_epoch_ms;

    Ok(Some(GooglePlayLifecycleOutcome {
        canonical_subscription: None,
        callback_event_type: Some("purchase.one_time".to_string()),
        callback_status_override: Some("refunded".to_string()),
        callback_revocation_reason_override: Some("REFUND".to_string()),
        callback_cancellation_mode_override: None,
    }))
}
