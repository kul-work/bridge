use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    application::app_context::{AppSnapshot, ProviderConfigSnapshot},
    db::{
        self,
        database::set_local_app_id,
        payments::{Payment, PaymentHistoryEntry},
        subscriptions::Subscription,
        webhooks::{WebhookDelivery, WebhookDeliveryEnqueue, WebhookRecord},
        Database,
    },
    error::BridgeError,
    ports::{
        traits::*,
        types::{
            OwnedWebhookPaymentRecord, SubscriptionLookupSnapshot, SubscriptionWebhookTransition,
            WebhookPaymentRecordRequest, WebhookProviderSnapshot, WebhookSubscriptionCommitRequest,
            WebhookSubscriptionSnapshot,
        },
        WebhookProcessingLookupRepository, WebhookProcessingMutationRepository,
        WebhookProcessingRepository, WebhookProcessingTransactionRepository,
    },
};

use super::{process_webhook, spawn_post_commit_effects, CanonicalWebhookPayload};

type ProcessingTx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

pub async fn process_webhook_atomically(
    database: &Database,
    app_id: Uuid,
    webhook_provider_id: Uuid,
    delivery_id: Uuid,
) -> Result<Option<CanonicalWebhookPayload>, BridgeError> {
    let mut tx = database
        .pool()
        .begin()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;

    let repo = AtomicWebhookProcessingRepository {
        database,
        tx: Arc::new(Mutex::new(Some(tx))),
    };

    let result = process_webhook(&repo, webhook_provider_id, app_id).await?;

    if let Some(processed) = result.as_ref() {
        let canonical_payload = serde_json::to_value(&processed.canonical)
            .map_err(|e| BridgeError::InternalServerError(e.to_string()))?;
        repo.store_webhook_delivery_canonical_payload_and_mark_processed(
            app_id,
            delivery_id,
            webhook_provider_id,
            canonical_payload,
        )
        .await?;
    }

    let tx = repo
        .tx
        .lock()
        .await
        .take()
        .ok_or_else(|| BridgeError::InternalServerError("webhook transaction already closed".to_string()))?;
    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    if let Some(processed) = result {
        spawn_post_commit_effects(processed.post_commit);
        return Ok(Some(processed.canonical));
    }

    Ok(None)
}

struct AtomicWebhookProcessingRepository<'a> {
    database: &'a Database,
    tx: Arc<Mutex<Option<ProcessingTx<'a>>>>,
}

impl<'a> AtomicWebhookProcessingRepository<'a> {
    async fn mark_webhook_processed_tx(&self, webhook_id: Uuid) -> Result<(), BridgeError> {
        let mut guard = self.tx.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| BridgeError::InternalServerError("webhook transaction closed".to_string()))?;
        sqlx::query("UPDATE pay.webhook_provider SET processed = true WHERE id = $1")
            .bind(webhook_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl<'a> AppLookupRepository for AtomicWebhookProcessingRepository<'a> {
    async fn get_app(&self, app_id: Uuid) -> Result<AppSnapshot, BridgeError> {
        self.database.get_app(app_id).await
    }
}

#[async_trait]
impl<'a> ProviderConfigLookupRepository for AtomicWebhookProcessingRepository<'a> {
    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfigSnapshot, BridgeError> {
        self.database.get_provider_config(app_id, provider).await
    }
}

impl<'a> AppConfigRepository for AtomicWebhookProcessingRepository<'a> {}

#[async_trait]
impl<'a> PaymentReadRepository for AtomicWebhookProcessingRepository<'a> {
    async fn count_user_payments(&self, app_id: Uuid, external_user_id: &str) -> Result<i64, BridgeError> {
        self.database.count_user_payments(app_id, external_user_id).await
    }

    async fn list_user_payments_keyset(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        after_created_at: Option<chrono::DateTime<chrono::Utc>>,
        after_id: Option<Uuid>,
    ) -> Result<Vec<PaymentHistoryEntry>, BridgeError> {
        self.database
            .list_user_payments_keyset(app_id, external_user_id, limit, after_created_at, after_id)
            .await
    }

    async fn get_user_payments(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Payment>, BridgeError> {
        self.database
            .get_user_payments(app_id, external_user_id, limit, offset)
            .await
    }

    async fn get_payment_status_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        self.database
            .get_payment_status_for_provider(app_id, provider, provider_transaction_id)
            .await
    }

    async fn get_payment_subscription_id_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        self.database
            .get_payment_subscription_id_for_provider(app_id, provider, provider_transaction_id)
            .await
    }

    async fn get_payment_currency_for_subscription(
        &self,
        app_id: Uuid,
        provider: &str,
        external_user_id: &str,
        subscription_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        self.database
            .get_payment_currency_for_subscription(app_id, provider, external_user_id, subscription_id)
            .await
    }
}

#[async_trait]
impl<'a> PaymentAcknowledgementRepository for AtomicWebhookProcessingRepository<'a> {
    async fn payment_acknowledged_at(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, BridgeError> {
        self.database
            .payment_acknowledged_at(app_id, provider, provider_transaction_id)
            .await
    }

    async fn mark_payment_acknowledged(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<(), BridgeError> {
        self.database
            .mark_payment_acknowledged(app_id, provider, provider_transaction_id)
            .await
    }
}

impl<'a> PaymentRepository for AtomicWebhookProcessingRepository<'a> {}

#[async_trait]
impl<'a> SubscriptionReadRepository for AtomicWebhookProcessingRepository<'a> {
    async fn get_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Subscription, BridgeError> {
        self.database
            .get_subscription(app_id, external_user_id, subscription_id, provider)
            .await
    }

    async fn get_user_subscriptions_keyset(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        cursor_created_at: Option<chrono::DateTime<chrono::Utc>>,
        cursor_id: Option<Uuid>,
    ) -> Result<Vec<Subscription>, BridgeError> {
        self.database
            .get_user_subscriptions_keyset(app_id, external_user_id, limit, cursor_created_at, cursor_id)
            .await
    }

    async fn get_user_subscriptions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Subscription>, BridgeError> {
        self.database
            .get_user_subscriptions(app_id, external_user_id, limit, offset)
            .await
    }
}

#[async_trait]
impl<'a> SubscriptionWriteRepository for AtomicWebhookProcessingRepository<'a> {
    async fn upsert_pending_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Subscription, BridgeError> {
        self.database
            .upsert_pending_subscription(app_id, external_user_id, subscription_id, provider)
            .await
    }

    async fn cancel_subscription_scheduled(&self, app_id: Uuid, id: Uuid) -> Result<Subscription, BridgeError> {
        self.database.cancel_subscription_scheduled(app_id, id).await
    }

    async fn cancel_subscription_immediate(&self, app_id: Uuid, id: Uuid) -> Result<Subscription, BridgeError> {
        self.database.cancel_subscription_immediate(app_id, id).await
    }

    async fn resume_subscription(&self, app_id: Uuid, id: Uuid) -> Result<Subscription, BridgeError> {
        self.database.resume_subscription(app_id, id).await
    }

    async fn mark_payment_acknowledged_for_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        subscription_id: &str,
        purchase_token: Option<&str>,
    ) -> Result<(), BridgeError> {
        self.database
            .mark_payment_acknowledged_for_subscription(
                app_id,
                external_user_id,
                provider,
                subscription_id,
                purchase_token,
            )
            .await
    }

    async fn accept_price_step_up(&self, app_id: Uuid, id: Uuid) -> Result<Subscription, BridgeError> {
        self.database.accept_price_step_up(app_id, id).await
    }

    async fn decline_price_step_up(&self, app_id: Uuid, id: Uuid) -> Result<Subscription, BridgeError> {
        self.database.decline_price_step_up(app_id, id).await
    }

    async fn clear_payment_failure_notification(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        subscription_id: &str,
    ) -> Result<(), BridgeError> {
        self.database
            .clear_payment_failure_notification(app_id, external_user_id, provider, subscription_id)
            .await
    }

    async fn delete_pending_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<(), BridgeError> {
        self.database
            .delete_pending_subscription(app_id, external_user_id, subscription_id, provider)
            .await
    }
}

#[async_trait]
impl<'a> SubscriptionLookupRepository for AtomicWebhookProcessingRepository<'a> {
    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
        self.database.get_subscription_by_sub_id(app_id, subscription_id).await
    }

    async fn get_subscription_by_sub_id_and_user(
        &self,
        app_id: Uuid,
        subscription_id: &str,
        external_user_id: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
        self.database
            .get_subscription_by_sub_id_and_user(app_id, subscription_id, external_user_id)
            .await
    }

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
        self.database
            .get_subscription_by_purchase_token(app_id, purchase_token)
            .await
    }
}

#[async_trait]
impl<'a> PurchaseOwnerLookupRepository for AtomicWebhookProcessingRepository<'a> {
    async fn lookup_user_by_subscription_id(
        &self,
        app_id: Uuid,
        provider: &str,
        subscription_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        self.database
            .lookup_user_by_subscription_id(app_id, provider, subscription_id)
            .await
    }

    async fn lookup_user_by_purchase_token(
        &self,
        app_id: Uuid,
        provider: &str,
        purchase_token: &str,
    ) -> Result<Option<String>, BridgeError> {
        self.database
            .lookup_user_by_purchase_token(app_id, provider, purchase_token)
            .await
    }

    async fn lookup_user_by_purchase_token_payment(
        &self,
        app_id: Uuid,
        provider: &str,
        purchase_token: &str,
    ) -> Result<Option<String>, BridgeError> {
        self.database
            .lookup_user_by_purchase_token_payment(app_id, provider, purchase_token)
            .await
    }

    async fn lookup_product_id_by_purchase_token_payment(
        &self,
        app_id: Uuid,
        provider: &str,
        purchase_token: &str,
    ) -> Result<Option<String>, BridgeError> {
        self.database
            .lookup_product_id_by_purchase_token_payment(app_id, provider, purchase_token)
            .await
    }
}

#[async_trait]
impl<'a> GooglePlayAccountLookupRepository for AtomicWebhookProcessingRepository<'a> {
    async fn lookup_user_by_google_obfuscated_id(
        &self,
        app_id: Uuid,
        obfuscated_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        self.database
            .lookup_user_by_google_obfuscated_id(app_id, obfuscated_id)
            .await
    }
}

impl<'a> SubscriptionRepository for AtomicWebhookProcessingRepository<'a> {}

#[async_trait]
impl<'a> WebhookReadRepository for AtomicWebhookProcessingRepository<'a> {
    async fn list_user_webhook_records(
        &self,
        app_id: Uuid,
        subscription_ids: &[String],
        purchase_tokens: &[String],
    ) -> Result<Vec<WebhookRecord>, BridgeError> {
        self.database
            .list_user_webhook_records(app_id, subscription_ids, purchase_tokens)
            .await
    }
}

#[async_trait]
impl<'a> WebhookWriteRepository for AtomicWebhookProcessingRepository<'a> {
    async fn create_webhook_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_webhook_id: &str,
        event_type: &str,
        subscription_id: Option<String>,
        purchase_token: Option<String>,
        payload: serde_json::Value,
        timestamp_epoch_ms: Option<i64>,
    ) -> Result<(Uuid, bool), BridgeError> {
        self.database
            .create_webhook_provider(
                app_id,
                provider,
                provider_webhook_id,
                event_type,
                subscription_id,
                purchase_token,
                payload,
                timestamp_epoch_ms,
            )
            .await
    }

    async fn create_webhook_delivery(
        &self,
        app_id: Uuid,
        webhook_provider_id: Uuid,
    ) -> Result<WebhookDeliveryEnqueue, BridgeError> {
        self.database
            .create_webhook_delivery(app_id, webhook_provider_id)
            .await
    }

    async fn store_webhook_delivery_canonical_payload_and_mark_processed(
        &self,
        app_id: Uuid,
        delivery_id: Uuid,
        webhook_provider_id: Uuid,
        canonical_payload: serde_json::Value,
    ) -> Result<(), BridgeError> {
        let mut guard = self.tx.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| BridgeError::InternalServerError("webhook transaction closed".to_string()))?;

        let result = sqlx::query(
            "UPDATE pay.webhook_delivery
             SET canonical_payload = COALESCE(canonical_payload, $1),
                 updated_at = NOW()
             WHERE id = $2
               AND app_id = $3
               AND webhook_provider_id = $4",
        )
        .bind(canonical_payload)
        .bind(delivery_id)
        .bind(app_id)
        .bind(webhook_provider_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| BridgeError::DbError(format!("Failed to store webhook delivery payload: {}", e)))?;

        if result.rows_affected() != 1 {
            return Err(BridgeError::ValidationError(
                "Webhook delivery not found for provider webhook".to_string(),
            ));
        }

        sqlx::query(
            "UPDATE pay.webhook_provider
             SET processed = true
             WHERE id = $1 AND app_id = $2",
        )
        .bind(webhook_provider_id)
        .bind(app_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| BridgeError::DbError(format!("Failed to mark webhook processed: {}", e)))?;

        Ok(())
    }

    async fn create_synthetic_webhook_delivery(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_webhook_id: &str,
        event_type: &str,
        subscription_id: Option<String>,
        purchase_token: Option<String>,
        provider_payload: serde_json::Value,
        timestamp_epoch_ms: Option<i64>,
        canonical_payload: serde_json::Value,
    ) -> Result<WebhookDeliveryEnqueue, BridgeError> {
        self.database
            .create_synthetic_webhook_delivery(
                app_id,
                provider,
                provider_webhook_id,
                event_type,
                subscription_id,
                purchase_token,
                provider_payload,
                timestamp_epoch_ms,
                canonical_payload,
            )
            .await
    }
}

#[async_trait]
impl<'a> WebhookSuppressionRepository for AtomicWebhookProcessingRepository<'a> {
    async fn suppress_webhook(&self, webhook_id: Uuid, reason: &str) -> Result<(), BridgeError> {
        let mut guard = self.tx.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| BridgeError::InternalServerError("webhook transaction closed".to_string()))?;
        sqlx::query(
            "UPDATE pay.webhook_provider
             SET suppressed = true, suppressed_reason = $1
             WHERE id = $2",
        )
        .bind(reason)
        .bind(webhook_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| BridgeError::DbError(format!("Failed to suppress webhook: {}", e)))?;
        Ok(())
    }
}

#[async_trait]
impl<'a> WebhookForwardRepository for AtomicWebhookProcessingRepository<'a> {
    async fn get_webhook_delivery(&self, id: Uuid) -> Result<WebhookDelivery, BridgeError> {
        self.database.get_webhook_delivery(id).await
    }

    async fn webhook_delivery_exists(&self, webhook_provider_id: Uuid) -> Result<bool, BridgeError> {
        self.database.webhook_delivery_exists(webhook_provider_id).await
    }

    async fn update_webhook_delivery_attempt(
        &self,
        delivery_id: Uuid,
        http_status: Option<i32>,
        error: Option<String>,
        forwarded: bool,
    ) -> Result<WebhookDelivery, BridgeError> {
        self.database
            .update_webhook_delivery_attempt(delivery_id, http_status, error, forwarded)
            .await
    }

    async fn reset_webhook_delivery(&self, delivery_id: Uuid) -> Result<bool, BridgeError> {
        self.database.reset_webhook_delivery(delivery_id).await
    }
}

#[async_trait]
impl<'a> WebhookProviderLookupRepository for AtomicWebhookProcessingRepository<'a> {
    async fn get_webhook_provider(&self, id: Uuid) -> Result<WebhookProviderSnapshot, BridgeError> {
        self.database.get_webhook_provider(id).await
    }
}

impl<'a> WebhookRepository for AtomicWebhookProcessingRepository<'a> {}

#[async_trait]
impl<'a> WebhookProcessingLookupRepository for AtomicWebhookProcessingRepository<'a> {
    async fn get_subscription_by_sub_id_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
        self.database
            .get_subscription_by_sub_id_for_provider(app_id, provider, subscription_id)
            .await
    }

    async fn get_subscription_by_sub_id_and_user_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        subscription_id: &str,
        external_user_id: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
        self.database
            .get_subscription_by_sub_id_and_user_for_provider(
                app_id,
                provider,
                subscription_id,
                external_user_id,
            )
            .await
    }

    async fn get_subscription_by_purchase_token_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        purchase_token: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
        self.database
            .get_subscription_by_purchase_token_for_provider(app_id, provider, purchase_token)
            .await
    }
}

#[async_trait]
impl<'a> WebhookProcessingMutationRepository for AtomicWebhookProcessingRepository<'a> {
    async fn update_payment_status_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
        new_status: &str,
    ) -> Result<(), BridgeError> {
        let mut guard = self.tx.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| BridgeError::InternalServerError("webhook transaction closed".to_string()))?;
        db::payments::update_payment_status_for_provider_tx(
            tx,
            app_id,
            provider,
            provider_transaction_id,
            new_status,
        )
        .await
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
        let mut guard = self.tx.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| BridgeError::InternalServerError("webhook transaction closed".to_string()))?;
        db::subscriptions::apply_webhook_transition_tx(
            tx,
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
        let mut guard = self.tx.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| BridgeError::InternalServerError("webhook transaction closed".to_string()))?;
        db::subscriptions::link_replacement_subscriptions_tx(
            tx,
            app_id,
            external_user_id,
            current_subscription_id,
            last_event_time,
        )
        .await
    }

    async fn mark_webhook_processed(&self, webhook_id: Uuid) -> Result<(), BridgeError> {
        self.mark_webhook_processed_tx(webhook_id).await
    }
}

#[async_trait]
impl<'a> WebhookProcessingTransactionRepository for AtomicWebhookProcessingRepository<'a> {
    async fn record_webhook_payment(
        &self,
        request: WebhookPaymentRecordRequest<'_>,
    ) -> Result<(), BridgeError> {
        let request = OwnedWebhookPaymentRecord::from(request);
        let mut guard = self.tx.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| BridgeError::InternalServerError("webhook transaction closed".to_string()))?;

        db::payments::record_payment_with_purchase_token_tx(
            tx,
            request.app_id,
            &request.external_user_id,
            &request.provider,
            &request.provider_transaction_id,
            request.provider_purchase_token.as_deref(),
            request.ack_required,
            request.subscription_id.as_deref(),
            request.product_id.as_deref(),
            request.amount_cents,
            request.currency.as_deref(),
            &request.status,
        )
        .await
    }

    async fn commit_webhook_subscription(
        &self,
        request: WebhookSubscriptionCommitRequest<'_>,
    ) -> Result<Option<WebhookSubscriptionSnapshot>, BridgeError> {
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
        let recurring_amount_cents = request.recurring_amount_cents;
        let payment = request.payment.map(OwnedWebhookPaymentRecord::from);
        let adopt_stale_payment = request.adopt_stale_payment;
        let stale_payment_window_secs = request.stale_payment_window_secs;

        let mut guard = self.tx.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| BridgeError::InternalServerError("webhook transaction closed".to_string()))?;

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
            recurring_amount_cents,
        )
        .await?;

        if !upsert_result.applied {
            return Ok(None);
        }

        if let Some(payment) = payment.as_ref() {
            db::payments::record_payment_with_purchase_token_tx(
                tx,
                payment.app_id,
                &payment.external_user_id,
                &payment.provider,
                &payment.provider_transaction_id,
                payment.provider_purchase_token.as_deref().or(purchase_token.as_deref()),
                payment.ack_required,
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

        Ok(Some(upsert_result.subscription.into()))
    }
}

impl<'a> WebhookProcessingRepository for AtomicWebhookProcessingRepository<'a> {}
