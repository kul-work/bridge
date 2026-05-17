use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{self, subscriptions::Subscription, webhooks::WebhookDelivery},
    error::BridgeError,
    ports::composites::{WebhookProcessingMutationRepository, WebhookProcessingTransactionRepository},
    ports::types::{OwnedWebhookPaymentRecord, SubscriptionWebhookTransition, TransactionOutcome, WebhookPaymentRecordRequest, WebhookSubscriptionCommitRequest},
    ports::traits::SchedulerRepository,
};

#[async_trait]
impl SchedulerRepository for db::Database {
    async fn list_enabled_apps(&self) -> Result<Vec<crate::db::apps::App>, BridgeError> {
        db::apps::list_enabled_apps(self.pool()).await
    }

    async fn list_pending_webhook_deliveries(
        &self,
        app_id: Uuid,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, BridgeError> {
        db::webhooks::list_pending_webhook_deliveries(self.pool(), app_id, limit).await
    }

    async fn list_reconciliation_subscriptions(
        &self,
        app_id: Uuid,
    ) -> Result<Vec<Subscription>, BridgeError> {
        db::subscriptions::list_reconciliation_subscriptions(self.pool(), app_id).await
    }

    async fn update_subscription_status(
        &self,
        app_id: Uuid,
        subscription_id: &str,
        new_status: &str,
        current_period_end: Option<chrono::DateTime<chrono::Utc>>,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError> {
        db::subscriptions::update_subscription_status(
            self.pool(),
            app_id,
            subscription_id,
            new_status,
            current_period_end,
            event_time_ms,
        )
        .await
    }

    async fn list_price_step_up_expired_subscriptions(
        &self,
        limit: i64,
    ) -> Result<Vec<Subscription>, BridgeError> {
        db::subscriptions::list_price_step_up_expired_subscriptions(self.pool(), limit).await
    }

    async fn mark_subscription_price_step_up_expired(
        &self,
        id: Uuid,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError> {
        db::subscriptions::mark_subscription_price_step_up_expired(self.pool(), id, event_time_ms).await
    }

    async fn list_pending_pause_subscriptions(
        &self,
        limit: i64,
    ) -> Result<Vec<Subscription>, BridgeError> {
        db::subscriptions::list_pending_pause_subscriptions(self.pool(), limit).await
    }

    async fn mark_subscription_paused(
        &self,
        id: Uuid,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError> {
        db::subscriptions::mark_subscription_paused(self.pool(), id, event_time_ms).await
    }

    async fn delete_orphaned_pending_subscriptions(&self) -> Result<u64, BridgeError> {
        db::subscriptions::delete_orphaned_pending_subscriptions(self.pool()).await
    }

    async fn cleanup_old_webhook_provider(&self) -> Result<(), BridgeError> {
        db::webhooks::cleanup_old_webhook_provider(self.pool()).await
    }

    async fn cleanup_purged_fraud_prevention(&self) -> Result<(), BridgeError> {
        db::users::cleanup_purged_fraud_prevention(self.pool()).await
    }
}

#[async_trait]
impl WebhookProcessingMutationRepository for db::Database {
    async fn update_payment_status_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
        new_status: &str,
    ) -> Result<(), BridgeError> {
        db::payments::update_payment_status_for_provider(self.pool(), app_id, provider, provider_transaction_id, new_status).await
    }

    async fn apply_subscription_transition(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        subscription_id: &str,
        event_time_ms: i64,
        transition: SubscriptionWebhookTransition,
    ) -> Result<Option<Subscription>, BridgeError> {
        db::subscriptions::apply_webhook_transition(
            self.pool(),
            app_id,
            external_user_id,
            provider,
            subscription_id,
            event_time_ms,
            transition,
        )
        .await
    }

    async fn link_replacement_subscriptions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        current_subscription_id: &str,
        last_event_time: i64,
    ) -> Result<(), BridgeError> {
        db::subscriptions::link_replacement_subscriptions(
            self.pool(),
            app_id,
            external_user_id,
            current_subscription_id,
            last_event_time,
        )
        .await
    }

    async fn mark_webhook_processed(&self, webhook_id: Uuid) -> Result<(), BridgeError> {
        db::webhooks::mark_webhook_processed(self.pool(), webhook_id).await
    }
}

#[async_trait]
impl WebhookProcessingTransactionRepository for db::Database {
    async fn record_webhook_payment(
        &self,
        request: WebhookPaymentRecordRequest<'_>,
    ) -> Result<(), BridgeError> {
        let request = OwnedWebhookPaymentRecord::from(request);
        let pool = self.pool();

        crate::ports::helpers::with_transaction_impl(pool, request.app_id, move |tx| {
            Box::pin(async move {
                db::payments::record_payment_tx(
                    tx,
                    request.app_id,
                    &request.external_user_id,
                    &request.provider,
                    &request.provider_transaction_id,
                    request.subscription_id.as_deref(),
                    request.product_id.as_deref(),
                    request.amount_cents,
                    request.currency.as_deref(),
                    &request.status,
                )
                .await?;

                Ok(TransactionOutcome::Commit(()))
            })
        })
        .await
    }

    async fn commit_webhook_subscription(
        &self,
        request: WebhookSubscriptionCommitRequest<'_>,
    ) -> Result<Option<crate::ports::types::WebhookSubscriptionSnapshot>, BridgeError> {
        let app_id = request.app_id;
        let external_user_id = request.external_user_id.to_string();
        let subscription_id = request.subscription_id.to_string();
        let provider = request.provider.to_string();
        let status = request.status.to_string();
        let current_period_end = request.current_period_end;
        let purchase_token = request.purchase_token.map(str::to_string);
        let auto_renewing = request.auto_renewing;
        let payment_state = request.payment_state;
        let provider_customer_id = request.provider_customer_id.map(str::to_string);
        let event_time_ms = request.event_time_ms;
        let payment = request.payment.map(OwnedWebhookPaymentRecord::from);
        let adopt_stale_payment = request.adopt_stale_payment;
        let stale_payment_window_secs = request.stale_payment_window_secs;

        let pool = self.pool();
        crate::ports::helpers::with_transaction_impl(pool, app_id, move |tx| {
            Box::pin(async move {
                let upsert_result = db::subscriptions::upsert_subscription_tx(
                    tx,
                    app_id,
                    &external_user_id,
                    &subscription_id,
                    &provider,
                    &status,
                    current_period_end,
                    purchase_token.as_deref(),
                    auto_renewing,
                    payment_state,
                    provider_customer_id.as_deref(),
                    event_time_ms,
                )
                .await?;

                if !upsert_result.applied {
                    return Ok(TransactionOutcome::Rollback(None));
                }

                if let Some(payment) = payment.as_ref() {
                    db::payments::record_payment_with_purchase_token_tx(
                        tx,
                        payment.app_id,
                        &payment.external_user_id,
                        &payment.provider,
                        &payment.provider_transaction_id,
                        purchase_token.as_deref(),
                        false,
                        payment.subscription_id.as_deref(),
                        payment.product_id.as_deref(),
                        payment.amount_cents,
                        payment.currency.as_deref(),
                        &payment.status,
                    )
                    .await?;
                }

                if adopt_stale_payment {
                    let _ = db::payments::adopt_stale_payment(
                        tx,
                        app_id,
                        &external_user_id,
                        &subscription_id,
                        stale_payment_window_secs,
                    )
                    .await;
                }

                Ok(TransactionOutcome::Commit(Some(upsert_result.subscription.into())))
            })
        })
        .await
    }
}
