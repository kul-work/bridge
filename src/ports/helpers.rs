use std::{future::Future, pin::Pin};
use uuid::Uuid;

use crate::{
    application::verify_purchase_types::VerifyPurchaseSubscriptionSnapshot,
    db::database::set_local_app_id,
    db::subscriptions::Subscription,
    error::BridgeError,
    ports::types::{SubscriptionLookupSnapshot, TransactionOutcome, WebhookPaymentRecordRequest, WebhookSubscriptionSnapshot, OwnedWebhookPaymentRecord},
};

impl<'a> From<WebhookPaymentRecordRequest<'a>> for OwnedWebhookPaymentRecord {
    fn from(request: WebhookPaymentRecordRequest<'a>) -> Self {
        Self {
            app_id: request.app_id,
            external_user_id: request.external_user_id.to_string(),
            provider: request.provider.to_string(),
            provider_transaction_id: request.provider_transaction_id.to_string(),
            subscription_id: request.subscription_id.map(str::to_string),
            product_id: request.product_id.map(str::to_string),
            amount_cents: request.amount_cents,
            currency: request.currency.map(str::to_string),
            status: request.status.to_string(),
        }
    }
}

impl From<Subscription> for WebhookSubscriptionSnapshot {
    fn from(subscription: Subscription) -> Self {
        Self {
            purchase_token: subscription.purchase_token,
            status: subscription.status,
            current_period_end: subscription.current_period_end,
            auto_renewing: subscription.auto_renewing,
            revocation_reason: subscription.revocation_reason,
            google_price_step_up_consent_deadline: subscription.google_price_step_up_consent_deadline,
            google_pause_scheduled_at: subscription.google_pause_scheduled_at,
            google_deferred_until: subscription.google_deferred_until,
        }
    }
}

pub(crate) fn map_subscription_lookup_snapshot(subscription: Subscription) -> SubscriptionLookupSnapshot {
    SubscriptionLookupSnapshot {
        id: subscription.id,
        external_user_id: subscription.external_user_id,
        provider: subscription.provider,
        subscription_id: subscription.subscription_id,
        purchase_token: subscription.purchase_token,
        status: subscription.status,
        current_period_end: subscription.current_period_end,
        auto_renewing: subscription.auto_renewing,
        revocation_reason: subscription.revocation_reason,
        last_event_time: subscription.last_event_time,
        google_price_step_up_consent_deadline: subscription.google_price_step_up_consent_deadline,
        google_pause_scheduled_at: subscription.google_pause_scheduled_at,
        google_deferred_until: subscription.google_deferred_until,
    }
}

pub(crate) fn map_verify_purchase_subscription(
    subscription: Subscription,
) -> VerifyPurchaseSubscriptionSnapshot {
    VerifyPurchaseSubscriptionSnapshot {
        current_period_end: subscription.current_period_end,
        auto_renewing: subscription.auto_renewing,
        payment_state: subscription.payment_state,
        provider_customer_id: subscription.provider_customer_id,
    }
}

pub(crate) fn with_transaction_impl<'a, T, F>(
    pool: &'a sqlx::PgPool,
    app_id: Uuid,
    f: F,
) -> Pin<Box<dyn Future<Output = Result<T, BridgeError>> + Send + 'a>>
where
    T: Send + 'a,
    F: for<'tx> FnOnce(
            &'tx mut sqlx::Transaction<'a, sqlx::Postgres>,
        ) -> Pin<Box<dyn Future<Output = Result<TransactionOutcome<T>, BridgeError>> + Send + 'tx>>
        + Send
        + 'a,
{
    Box::pin(async move {
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;

        set_local_app_id(&mut tx, app_id).await?;

        let outcome = f(&mut tx).await?;

        match outcome {
            TransactionOutcome::Commit(value) => {
                tx.commit()
                    .await
                    .map_err(|e| BridgeError::DbError(e.to_string()))?;
                Ok(value)
            }
            TransactionOutcome::Rollback(value) => {
                tx.rollback()
                    .await
                    .map_err(|e| BridgeError::DbError(e.to_string()))?;
                Ok(value)
            }
        }
    })
}
